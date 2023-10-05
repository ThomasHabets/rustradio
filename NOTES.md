# Internal rustradio notes

## Things to improve

### API

* Not great that it needs a macro for sample-at-a-time.
* Stream type safety is done at runtime.
* No clean way to "just get the input"
* Iter is nice, but should we use circular ring buffer for efficiency?
* InputStreams.get() and OutputStreams.get() both return Streamp<T>. Not type safe.

### Internal

* Do we really need refcounted streams?
* Run multithreaded.
