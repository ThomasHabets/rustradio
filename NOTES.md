# Internal rustradio notes

## Things to improve

### API

* I'm not entirely happy with the work function and buffer interfaces.
* Probably the `process_sync_tags()` functions should get all tag inputs, and
  return them too. Both not just for the first arg.
* Think about if Cow could improve things.
* New builder pattern `build_with_optional()` that returns a struct of all
  optional streams, but as `Option<>` where only the requested streams are
  actually created?
* Dropping optional dst should gracefully stop producing on it?

### Internal

* Do we really need refcounted streams?

### New block ideas

* Combine Multiply with SignalSourceComplex into MultiplyBySignalSource or
  something, since it's a common op that doesn't need joins. Or maybe build it
  into Vco, by changing the amplitude via tags?
