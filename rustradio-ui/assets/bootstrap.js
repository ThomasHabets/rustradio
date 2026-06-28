// Minimal necessary code to start a worker sharing RAM with the main UI thread.
export async function bootstrap({pkgName, wasmMemoryConfig, workerThreadStackSize}) {
  import init, { start, initThreadPool } from `./${pkgName}.js`;
  Error.stackTraceLimit = Infinity;
  const wasmUrl = new URL(`${pkgName}_bg.wasm`, import.meta.url);
  
  function assertSharedMemoryAvailable() {
    if (typeof SharedArrayBuffer === "undefined") {
      throw new Error(
        "SharedArrayBuffer is unavailable. Serve this app with Cross-Origin-Opener-Policy: same-origin and Cross-Origin-Embedder-Policy: require-corp.",
      );
    }
  }
  
  async function compileWasmModule() {
    try {
      return await WebAssembly.compileStreaming(fetch(wasmUrl));
    } catch {
      const response = await fetch(wasmUrl);
      return await WebAssembly.compile(await response.arrayBuffer());
    }
  }
  
  assertSharedMemoryAvailable();
  
  if (globalThis.window) {
    const memory = new WebAssembly.Memory(wasmMemoryConfig);
    const module = await compileWasmModule();
    globalThis.__ruwasmMemory = memory;
    globalThis.__ruwasmModule = module;
    await init({ module_or_path: module, memory });
    console.log("About to init thread pool");
    await initThreadPool(navigator.hardwareConcurrency);
    console.log("Thread pool inited");
    await start();
  } else {
    globalThis.addEventListener("message", async function onInit(event) {
      try {
        globalThis.postMessage({ type: "ruwasm-bootstrap-init-received" });
        const message = event.data;
        if (
          message?.type !== "ruwasm-init" ||
          !(message.memory instanceof WebAssembly.Memory) ||
          !(message.module instanceof WebAssembly.Module)
        ) {
          throw new Error("worker received invalid ruwasm init message");
        }
  
        const memory = message.memory;
        const module = message.module;
        globalThis.__ruwasmMemory = memory;
        globalThis.__ruwasmModule = module;
        await init({ module_or_path: module, memory, thread_stack_size: workerThreadStackSize });
        globalThis.postMessage({ type: "ruwasm-bootstrap-init-complete" });
        await start();
      } catch (error) {
        globalThis.postMessage({
          type: "ruwasm-bootstrap-error",
          message: String(error),
          stack: error?.stack ?? "",
        });
        throw error;
      }
    }, { once: true });
    globalThis.postMessage({ type: "ruwasm-bootstrap-ready" });
  }
}
