# Data stream thoughts

It looks like there's no built in flow control in websockets. We therefore need
to build some sort of windowing ourselves on the communication from websocket,
to main thread, to worker.

## Requirements

* Performant. Probably want to provide a "receiver window" kind of thing, with
  periodic updates.
* Multi-stream. The WASM may only be a UI for a whole flowgraph that runs
  native.
* Bidirectional. Browser audio or whatever, could be required to feed back.
* Support "messages" too, for when the UI needs to tell the websocket server to
  change frequency.

## Protocol thoughts

* TLV. Use little endian, since it's more common. Each packet is prefixed by a
  32bit length, then its type, then type-specific data.
* Before any other packet is sent, both sides need to send a "version" packet.
  We start off with version 0. That packet is therefore the u32 number 2, the
  u8 number 1 (for version), and then the u32 number 0 (numeric version).
* Data is always pulled, by sending a RequestData packet, with a "send me up to
  this much data" window, and a string stream identifier. So that packet is u32
  number 1+4+string_len, packet type 2 for RequestData, u32 for window size, and
  the rest is the stream identifier.
* When there's data, the other side then sends it, if it fits in the announced
  window, under packet type 3.
* At any point the RequestData may be re-sent with a new window, bigger or
  smaller, and the sender then updates it on its side, sending either more, or
  stopping the send.
