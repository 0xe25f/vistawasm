// Shared by the visual-check page (`index.html`) and `demo-capture.mjs`,
// which adds it to the demo with Playwright's `addInitScript`.
// Software WebGPU in headless Chromium loses the device when it
// presents to a real canvas, so frames are drawn into an ordinary
// texture that `capture()` reads back instead.
(() => {
  const proto = GPUCanvasContext.prototype;
  proto.configure = function (descriptor) {
    window.gpuDevice = descriptor.device;
    window.gpuFormat = descriptor.format;
    window.gpuCanvas = this.canvas;
    window.gpuTexture = null;
  };
  proto.getCurrentTexture = function () {
    const canvas = window.gpuCanvas;
    let texture = window.gpuTexture;

    if (!texture || texture.width !== canvas.width || texture.height !== canvas.height) {
      texture = window.gpuDevice.createTexture({
        size: [canvas.width, canvas.height],
        format: window.gpuFormat,
        usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC | GPUTextureUsage.TEXTURE_BINDING
      });
      window.gpuTexture = texture;
    }

    return texture;
  };
})();
