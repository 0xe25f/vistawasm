// The smallest useful three-terrain app: voxelise a generated heightmap
// and render it. three-terrain has no generator of its own, so the
// heightmap is filled with a few octaves of sine noise.
import { HemisphereLight, PerspectiveCamera, Scene, WebGLRenderer } from "three";
import Terrain from "three-terrain";

const canvas = document.querySelector("#view");
const renderer = new WebGLRenderer({ canvas });
const scene = new Scene();
const camera = new PerspectiveCamera(55, 960 / 540, 1, 5000);
camera.position.set(0, 300, 700);
camera.lookAt(0, 60, 0);
scene.add(new HemisphereLight(0xffffff, 0x445544, 1));

const terrain = new Terrain({
  width: 512,
  height: 255,
  depth: 512,
  generator: (target) => {
    const heights = target.mesher.memory.heightmap.view;

    for (let z = 0; z < 512; z += 1) {
      for (let x = 0; x < 512; x += 1) {
        heights[z * 512 + x] = 0.5 + 0.25 * Math.sin(x * 0.02) * Math.cos(z * 0.017);
      }
    }
  }
});
scene.add(terrain);
renderer.setAnimationLoop(() => renderer.render(scene, camera));
