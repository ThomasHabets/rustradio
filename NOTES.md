# Internal rustradio notes

## Things to improve

### API

* I'm not entirely happy with the work function and buffer interfaces.
* Probably the `process_sync_tags()` functions should get all tag inputs, and
  return them too. Both not just for the first arg.
* Think about if Cow could improve things.
* Should blocks return weak Streamp's? Not sure. Because maybe the
  owning block gets dropped while there's still data in the Stream.
* Some static way to prevent multiple consumers, or support multiple
  consumers in a correct way.

### Internal

* Do we really need refcounted streams?
* Multithreaded graphs should condvar sleep if they need more input, or more
  output space.
