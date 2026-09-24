//! GPU compute erosion for browser builds: stage D of fractal generation.
//!
//! Runs the passes in `shaders/hydraulic_erosion.wgsl` and
//! `shaders/thermal_erosion.wgsl` with the same schedule, constants and
//! cell-unit heights as the CPU reference in `terrain/erosion.rs`: 60 % of
//! the iterations at half resolution, the change upsampled onto the full
//! map, then the remaining 40 % at full resolution. Every pass writes only
//! its own cell, so no synchronisation is needed inside a dispatch.
//!
//! Work is submitted in chunks, and each chunk's completion is awaited
//! before the next is queued, so progress is reported at least every
//! 10 % and the page stays responsive. Results can differ from the CPU
//! reference in the last bits (floating-point order differs), but follow
//! it pass for pass.

use bytemuck::{Pod, Zeroable};
use vista_types::ErosionOptions;

use crate::errors::{VistaError, VistaResult};
use crate::terrain::erosion::{
  downsample, erosion_iterations, erosion_schedule, rain_weights, upsample, ErosionIterations,
  ErosionParams, CREEP_RATE, DEPOSIT_RATE, DISSOLVE_RATE, LEVEL_SINE, MIN_TILT, PIPE_GAIN,
  THERMAL_RATE,
};
use crate::terrain::fractal::Progress;
use crate::terrain::heightmap::HeightMap;
use crate::terrain::landforms::Landform;

/// Mirrors `ErosionParams` in the WGSL.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ErosionUniform {
  size: u32,
  pipe_gain: f32,
  min_tilt: f32,
  level_sine: f32,
  dissolve_rate: f32,
  deposit_rate: f32,
  thermal_rate: f32,
  rain: f32,
  evaporation: f32,
  capacity: f32,
  full_depth: f32,
  talus: f32,
  creep_rate: f32,
  _padding: [f32; 3],
}

const _: () = assert!(std::mem::size_of::<ErosionUniform>() == 64);

impl ErosionUniform {
  fn new(size: u32, params: &ErosionParams) -> Self {
    Self {
      size,
      pipe_gain: PIPE_GAIN,
      min_tilt: MIN_TILT,
      level_sine: LEVEL_SINE,
      dissolve_rate: DISSOLVE_RATE,
      deposit_rate: DEPOSIT_RATE,
      thermal_rate: THERMAL_RATE,
      rain: params.rain,
      evaporation: params.evaporation,
      capacity: params.capacity,
      full_depth: params.full_depth,
      talus: params.talus,
      creep_rate: CREEP_RATE,
      _padding: [0.0; 3],
    }
  }
}

const WORKGROUP_SIZE: u32 = 8;

/// Hydraulic passes in the order one iteration runs them.
const HYDRAULIC_PASSES: [&str; 6] = [
  "rain",
  "outflow",
  "update_water",
  "erode",
  "advect",
  "evaporate",
];

/// GPU compute pipelines for erosion.
///
/// Created once per `GpuContext` and reused for every terrain generation,
/// since the pipelines do not depend on terrain size.
pub struct ErosionCompute {
  bind_group_layout: wgpu::BindGroupLayout,
  hydraulic: Vec<wgpu::ComputePipeline>,
  settle: wgpu::ComputePipeline,
  thermal: [wgpu::ComputePipeline; 2],
}

/// Buffers for one grid size.
struct Field {
  size: u32,
  terrain: wgpu::Buffer,
  bind_group: wgpu::BindGroup,
}

impl ErosionCompute {
  /// Compile the erosion compute pipelines.
  pub fn new(device: &wgpu::Device) -> Self {
    let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
      binding,
      visibility: wgpu::ShaderStages::COMPUTE,
      ty: wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: false },
        has_dynamic_offset: false,
        min_binding_size: None,
      },
      count: None,
    };
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
      label: Some("VistaWASM erosion bind group layout"),
      entries: &[
        wgpu::BindGroupLayoutEntry {
          binding: 0,
          visibility: wgpu::ShaderStages::COMPUTE,
          ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
          },
          count: None,
        },
        storage(1),
        storage(2),
        storage(3),
        storage(4),
        storage(5),
        storage(6),
      ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
      label: Some("VistaWASM erosion pipeline layout"),
      bind_group_layouts: &[Some(&bind_group_layout)],
      immediate_size: 0,
    });
    let hydraulic_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
      label: Some("VistaWASM hydraulic erosion shader"),
      source: wgpu::ShaderSource::Wgsl(crate::render::shaders::HYDRAULIC_EROSION.into()),
    });
    let thermal_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
      label: Some("VistaWASM thermal erosion shader"),
      source: wgpu::ShaderSource::Wgsl(crate::render::shaders::THERMAL_EROSION.into()),
    });
    let pipeline = |module: &wgpu::ShaderModule, entry: &str| {
      device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("VistaWASM erosion pipeline"),
        layout: Some(&pipeline_layout),
        module,
        entry_point: Some(entry),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
      })
    };

    Self {
      hydraulic: HYDRAULIC_PASSES
        .iter()
        .map(|entry| pipeline(&hydraulic_shader, entry))
        .collect(),
      settle: pipeline(&hydraulic_shader, "settle"),
      thermal: [
        pipeline(&thermal_shader, "exchange"),
        pipeline(&thermal_shader, "apply"),
      ],
      bind_group_layout,
    }
  }

  /// Erode `map` on the GPU and return the eroded heights in metres, with
  /// the landform's defaults for unset options. Reports progress as the
  /// `"erosion"` phase.
  pub async fn run(
    &self,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    map: &HeightMap,
    options: &ErosionOptions,
    landform: &Landform,
    progress: Progress<'_>,
  ) -> VistaResult<Vec<f32>> {
    let size = map.metadata.width as usize;
    let mut heights = map.heights.clone();

    if size != map.metadata.height as usize || size < 4 {
      return Ok(heights);
    }

    let iterations = erosion_iterations(options);
    let total = iterations.hydraulic.max(iterations.thermal).max(1);
    // At least ten reports, whatever the iteration count.
    let chunk = total.div_ceil(10).max(1);
    let mut done = 0;
    let metres = map.metadata.metres_per_sample;
    progress("erosion", 0.0);

    for phase in erosion_schedule(iterations, size as u32) {
      let (grid, cell) = if phase.half {
        (size / 2, metres * 2.0)
      } else {
        (size, metres)
      };
      let before = if phase.half {
        downsample(&heights, size)
      } else {
        heights.clone()
      };
      let terrain: Vec<f32> = before.iter().map(|h| h / cell).collect();
      let params = ErosionParams::new(options, landform, cell);
      let field = self.field(
        device,
        queue,
        grid as u32,
        &terrain,
        &rain_weights(map, grid as u32),
        &params,
      );
      let mut step = 0;
      let steps = phase.iterations.hydraulic.max(phase.iterations.thermal);

      while step < steps {
        let count = chunk.min(steps - step);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
          label: Some("VistaWASM erosion chunk"),
        });
        self.encode_iterations(&mut encoder, &field, step, count, phase.iterations);

        if step + count == steps {
          self.dispatch(&mut encoder, &field, &self.settle);
        }

        queue.submit(Some(encoder.finish()));
        work_done(queue).await?;
        step += count;
        done += count;
        progress("erosion", (done as f32 / total as f32).min(1.0));
      }

      let eroded = read_terrain(device, queue, &field).await?;

      if phase.half {
        let change: Vec<f32> = eroded
          .iter()
          .zip(&before)
          .map(|(value, original)| value * cell - original)
          .collect();

        for (height, delta) in heights.iter_mut().zip(upsample(&change, size)) {
          *height += delta;
        }
      } else {
        heights = eroded.iter().map(|value| value * cell).collect();
      }
    }

    Ok(heights)
  }

  fn field(
    &self,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: u32,
    terrain: &[f32],
    weights: &[f32],
    params: &ErosionParams,
  ) -> Field {
    let count = (size * size) as u64;
    let buffer = |label: &str, bytes: u64| {
      device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
          | wgpu::BufferUsages::COPY_SRC
          | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
      })
    };
    // New buffers start zeroed, which is the dry, sediment-free state.
    let terrain_buffer = buffer("VistaWASM erosion terrain", count * 4);
    let water = buffer("VistaWASM erosion water", count * 4);
    let sediment = buffer("VistaWASM erosion sediment", count * 4);
    let scratch = buffer("VistaWASM erosion scratch", count * 4);
    let flux = buffer("VistaWASM erosion flux", count * 16);
    let velocity = buffer("VistaWASM erosion velocity", count * 16);
    let packed: Vec<[f32; 4]> = weights.iter().map(|w| [0.0, 0.0, 0.0, *w]).collect();
    queue.write_buffer(&terrain_buffer, 0, bytemuck::cast_slice(terrain));
    queue.write_buffer(&velocity, 0, bytemuck::cast_slice(&packed));

    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM erosion params"),
      size: std::mem::size_of::<ErosionUniform>() as u64,
      usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    queue.write_buffer(
      &uniform,
      0,
      bytemuck::bytes_of(&ErosionUniform::new(size, params)),
    );

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM erosion bind group"),
      layout: &self.bind_group_layout,
      entries: &[
        wgpu::BindGroupEntry {
          binding: 0,
          resource: uniform.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 1,
          resource: terrain_buffer.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 2,
          resource: water.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 3,
          resource: sediment.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 4,
          resource: scratch.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 5,
          resource: flux.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 6,
          resource: velocity.as_entire_binding(),
        },
      ],
    });

    Field {
      size,
      terrain: terrain_buffer,
      bind_group,
    }
  }

  /// Encode iterations `first .. first + count`, interleaving thermal
  /// steps with hydraulic ones as the CPU reference does.
  fn encode_iterations(
    &self,
    encoder: &mut wgpu::CommandEncoder,
    field: &Field,
    first: u32,
    count: u32,
    iterations: ErosionIterations,
  ) {
    let groups = field.size.div_ceil(WORKGROUP_SIZE);
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
      label: Some("VistaWASM erosion pass"),
      timestamp_writes: None,
    });
    pass.set_bind_group(0, &field.bind_group, &[]);

    for step in first..first + count {
      if step < iterations.hydraulic {
        for pipeline in &self.hydraulic {
          pass.set_pipeline(pipeline);
          pass.dispatch_workgroups(groups, groups, 1);
        }
      }

      if step < iterations.thermal {
        for pipeline in &self.thermal {
          pass.set_pipeline(pipeline);
          pass.dispatch_workgroups(groups, groups, 1);
        }
      }
    }
  }

  fn dispatch(
    &self,
    encoder: &mut wgpu::CommandEncoder,
    field: &Field,
    pipeline: &wgpu::ComputePipeline,
  ) {
    let groups = field.size.div_ceil(WORKGROUP_SIZE);
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
      label: Some("VistaWASM erosion settle pass"),
      timestamp_writes: None,
    });
    pass.set_bind_group(0, &field.bind_group, &[]);
    pass.set_pipeline(pipeline);
    pass.dispatch_workgroups(groups, groups, 1);
  }
}

/// Wait until the GPU has finished everything submitted so far.
async fn work_done(queue: &wgpu::Queue) -> VistaResult<()> {
  let (sender, receiver) = futures_channel::oneshot::channel();
  queue.on_submitted_work_done(move || {
    let _ = sender.send(());
  });
  receiver.await.map_err(|_| {
    VistaError::internal("The GPU erosion work was dropped before it finished.".to_string())
  })
}

/// Copy the field's terrain back to the CPU.
async fn read_terrain(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  field: &Field,
) -> VistaResult<Vec<f32>> {
  let len = (field.size * field.size) as usize;
  let bytes = len as u64 * 4;
  let staging = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some("VistaWASM erosion staging buffer"),
    size: bytes,
    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    mapped_at_creation: false,
  });
  let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
    label: Some("VistaWASM erosion readback"),
  });
  encoder.copy_buffer_to_buffer(&field.terrain, 0, &staging, 0, bytes);
  queue.submit(Some(encoder.finish()));

  let slice = staging.slice(..);
  let (sender, receiver) = futures_channel::oneshot::channel();
  slice.map_async(wgpu::MapMode::Read, move |result| {
    let _ = sender.send(result);
  });

  match receiver.await {
    Ok(Ok(())) => {}
    _ => {
      return Err(VistaError::internal(
        "Could not map the GPU erosion result buffer for readback.".to_string(),
      ));
    }
  }

  let view = slice
    .get_mapped_range()
    .map_err(|error| VistaError::internal(error.to_string()))?;
  let mut result = vec![0.0_f32; len];
  result.copy_from_slice(bytemuck::cast_slice(&view));
  drop(view);
  staging.unmap();

  Ok(result)
}
