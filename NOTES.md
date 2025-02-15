# Internal rustradio notes

## Things to improve

### API

* I'm not entirely happy with the work function and buffer interfaces.
* Probably the `process_sync_tags()` functions should get all tag inputs, and
  return them too. Both not just for the first arg.
* Think about if Cow could improve things.

### Internal

* Do we really need refcounted streams?
