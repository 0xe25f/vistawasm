import { rmSync } from "node:fs";

for (const path of ["dist", "js/pkg"]) {
  rmSync(path, {
    force: true,
    recursive: true
  });
}
