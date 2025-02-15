# Internal rustradio notes

## Things to improve

### API

* I'm not entirely happy with the work function and buffer interfaces.
* Probably the `process_sync_tags()` functions should get all tag inputs, and
  return them too. Both not just for the first arg.
* Think about if Cow could improve things.
* Should blocks return weak Streamp's? Not sure. Because maybe the
  owning block gets dropped while there's still data in the Stream.
* Need a way for a block to say Noop until there's more data on a stream, or
  room on output.
  * Graph can then check that before calling `work`.
  * MTGraph could condvar it.

### Internal

* Do we really need refcounted streams?
* Multithreaded graphs should condvar sleep if they need more input, or more
  output space.
