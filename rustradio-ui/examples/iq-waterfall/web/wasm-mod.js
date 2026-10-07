import { bootstrap } from "./rustradio-ui-bootstrap.js";
await bootstrap({
  pkgName: "iq_waterfall",
  wasmMemoryConfig: { initial: 64, maximum: 16384, shared: true },
  workerThreadStackSize: 1024 * 1024,
});
