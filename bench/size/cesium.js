// The smallest useful CesiumJS app: serve a height field as terrain and
// render it on the globe, with Cesium's own sky and atmosphere. There is
// no imagery, so nothing is fetched from Cesium ion.
import { CesiumWidget, Color, JulianDate } from "cesium";
import "cesium/Build/Cesium/Widgets/widgets.css";
import { VIEW, fieldProvider } from "./cesium-field.js";

const widget = new CesiumWidget(document.querySelector("#view"), {
  baseLayer: false,
  terrainProvider: fieldProvider()
});
// Without imagery the globe is drawn in its base colour.
widget.scene.globe.baseColor = Color.fromCssColorString("#5c8a4a");
widget.scene.globe.enableLighting = true;
// Late morning, so the sun lights the terrain whenever the app is opened.
widget.clock.currentTime = JulianDate.fromIso8601("2026-06-21T09:00:00Z");
widget.clock.shouldAnimate = false;
widget.camera.setView(VIEW);
window.cesiumWidget = widget;
