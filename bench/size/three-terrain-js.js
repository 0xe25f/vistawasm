// The smallest useful THREE.Terrain app: generate a terrain and render it.
import * as THREE from "three";
import Terrain, { TerrainNS, createSeededRandom } from "three.terrain.js";

const canvas = document.querySelector("#view");
const renderer = new THREE.WebGLRenderer({ canvas });
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(55, 960 / 540, 1, 20000);
camera.position.set(0, 600, 2400);
camera.lookAt(0, 150, 0);
scene.add(new THREE.HemisphereLight(0xffffff, 0x445544, 1));

const terrain = Terrain({
  heightmap: TerrainNS.SimplexLayers,
  material: new THREE.MeshLambertMaterial({ color: 0x5c8a4a }),
  maxHeight: 300,
  minHeight: -100,
  random: createSeededRandom(1),
  xSegments: 511,
  xSize: 6132,
  ySegments: 511,
  ySize: 6132
});
scene.add(terrain);
renderer.setAnimationLoop(() => renderer.render(scene, camera));
