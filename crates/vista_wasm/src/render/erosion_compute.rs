//! GPU compute erosion for browser builds.
//!
//! Hydraulic and thermal erosion run as WGSL compute passes over a
//! ping-pong pair of storage buffers, so large terrain and high iteration
//! counts stay fast. Both passes are written as a "gather": every
//! invocation reads its own cell and its direct neighbours from the input
//! buffer and writes only its own cell in the output buffer, so iterations
//! are race-free and require no manual synchronisation between
//! invocations.
//!
//! This module intentionally does not attempt to match the CPU reference
//! erosion in `terrain/erosion.rs` bit-for-bit. Both are tuned to the same
//! public options and produce comparable results, but small numerical
//! differences between the CPU and GPU paths are expected and acceptable,
//! as documented in the project plan.

use bytemuck::{Pod, Zeroable};
use vista_types::{ErosionOptions, ErosionQuality};

use crate::errors::{VistaError, VistaResult};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HydraulicParams {
  width: u32,
  height: u32,
  transfer_rate: f32,
  _padding: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ThermalParams {
  width: u32,
  height: u32,
  talus_threshold: f32,
  transfer_rate: f32,
}

/// GPU compute pipelines for hydraulic and thermal erosion.
///
/// Created once per `GpuContext` and reused for every terrain generation,
/// since the pipelines do not depend on terrain size.
pub struct ErosionCompute {
  bind_group_layout: wgpu::BindGroupLayout,
  hydraulic_pipeline: wgpu::ComputePipeline,
  thermal_pipeline: wgpu::ComputePipeline,
}

const WORKGROUP_SIZE: u32 = 8;

impl ErosionCompute {
  /// Compile the erosion compute pipelines.
  pub fn new(device: &wgpu::Device) -> Self {
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
        wgpu::BindGroupLayoutEntry {
          binding: 1,
          visibility: wgpu::ShaderStages::COMPUTE,
          ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
          },
          count: None,
        },
        wgpu::BindGroupLayoutEntry {
          binding: 2,
          visibility: wgpu::ShaderStages::COMPUTE,
          ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
          },
          count: None,
        },
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

    let hydraulic_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
      label: Some("VistaWASM hydraulic erosion pipeline"),
      layout: Some(&pipeline_layout),
      module: &hydraulic_shader,
      entry_point: Some("main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      cache: None,
    });
    let thermal_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
      label: Some("VistaWASM thermal erosion pipeline"),
      layout: Some(&pipeline_layout),
      module: &thermal_shader,
      entry_point: Some("main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      cache: None,
    });

    Self {
      bind_group_layout,
      hydraulic_pipeline,
      thermal_pipeline,
    }
  }

  /// Run budgeted hydraulic and thermal erosion on the GPU and return the
  /// eroded heights. Returns the input heights unchanged if both iteration
  /// counts are zero.
  pub async fn run(
    &self,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    heights: &[f32],
    width: u32,
    height: u32,
    metres_per_sample: f32,
    options: &ErosionOptions,
  ) -> VistaResult<Vec<f32>> {
    let budget = match options.quality.unwrap_or(ErosionQuality::Preview) {
      ErosionQuality::Preview => 16,
      ErosionQuality::Balanced => 64,
      ErosionQuality::High => 160,
      ErosionQuality::Offline => 320,
    };
    let hydraulic_iterations = options.hydraulic_iterations.unwrap_or(0).min(budget);
    let thermal_iterations = options.thermal_iterations.unwrap_or(0).min(budget);

    if hydraulic_iterations == 0 && thermal_iterations == 0 {
      return Ok(heights.to_vec());
    }

    let byte_len = (width as u64) * (height as u64) * 4;

    let buffer_a = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM erosion buffer A"),
      size: byte_len,
      usage: wgpu::BufferUsages::STORAGE
        | wgpu::BufferUsages::COPY_SRC
        | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    let buffer_b = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM erosion buffer B"),
      size: byte_len,
      usage: wgpu::BufferUsages::STORAGE
        | wgpu::BufferUsages::COPY_SRC
        | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    queue.write_buffer(&buffer_a, 0, bytemuck::cast_slice(heights));

    let hydraulic_rate = options.rain_amount.unwrap_or(0.02).clamp(0.0, 1.0)
      * options.sediment_capacity.unwrap_or(0.04).clamp(0.0, 1.0);
    let hydraulic_uniform = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM hydraulic erosion params"),
      size: std::mem::size_of::<HydraulicParams>() as u64,
      usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    queue.write_buffer(
      &hydraulic_uniform,
      0,
      bytemuck::bytes_of(&HydraulicParams {
        width,
        height,
        transfer_rate: hydraulic_rate,
        _padding: 0,
      }),
    );

    let talus_threshold = options
      .talus_angle_degrees
      .unwrap_or(35.0)
      .to_radians()
      .tan()
      * metres_per_sample.max(0.001)
      * 0.15;
    let thermal_uniform = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM thermal erosion params"),
      size: std::mem::size_of::<ThermalParams>() as u64,
      usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    queue.write_buffer(
      &thermal_uniform,
      0,
      bytemuck::bytes_of(&ThermalParams {
        width,
        height,
        talus_threshold,
        transfer_rate: 0.08,
      }),
    );

    let hydraulic_a_to_b = self.bind_group(device, &hydraulic_uniform, &buffer_a, &buffer_b);
    let hydraulic_b_to_a = self.bind_group(device, &hydraulic_uniform, &buffer_b, &buffer_a);
    let thermal_a_to_b = self.bind_group(device, &thermal_uniform, &buffer_a, &buffer_b);
    let thermal_b_to_a = self.bind_group(device, &thermal_uniform, &buffer_b, &buffer_a);

    let workgroups_x = (width + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
    let workgroups_y = (height + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
      label: Some("VistaWASM erosion compute"),
    });
    let mut result_is_a = true;

    if hydraulic_iterations > 0 {
      let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("VistaWASM hydraulic erosion pass"),
        timestamp_writes: None,
      });
      pass.set_pipeline(&self.hydraulic_pipeline);

      for _ in 0..hydraulic_iterations {
        let bind_group = if result_is_a {
          &hydraulic_a_to_b
        } else {
          &hydraulic_b_to_a
        };
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
        result_is_a = !result_is_a;
      }
    }

    if thermal_iterations > 0 {
      let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("VistaWASM thermal erosion pass"),
        timestamp_writes: None,
      });
      pass.set_pipeline(&self.thermal_pipeline);

      for _ in 0..thermal_iterations {
        let bind_group = if result_is_a {
          &thermal_a_to_b
        } else {
          &thermal_b_to_a
        };
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
        result_is_a = !result_is_a;
      }
    }

    let result_buffer = if result_is_a { &buffer_a } else { &buffer_b };
    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM erosion staging buffer"),
      size: byte_len,
      usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
      mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(result_buffer, 0, &staging_buffer, 0, byte_len);
    queue.submit(Some(encoder.finish()));

    read_buffer_f32(&staging_buffer, (width as usize) * (height as usize)).await
  }

  fn bind_group(
    &self,
    device: &wgpu::Device,
    params: &wgpu::Buffer,
    input: &wgpu::Buffer,
    output: &wgpu::Buffer,
  ) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM erosion bind group"),
      layout: &self.bind_group_layout,
      entries: &[
        wgpu::BindGroupEntry {
          binding: 0,
          resource: params.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 1,
          resource: input.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 2,
          resource: output.as_entire_binding(),
        },
      ],
    })
  }
}

/// Map a buffer for reading and copy its contents into a `Vec<f32>`.
async fn read_buffer_f32(buffer: &wgpu::Buffer, len: usize) -> VistaResult<Vec<f32>> {
  let slice = buffer.slice(..);
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
  buffer.unmap();

  Ok(result)
}
