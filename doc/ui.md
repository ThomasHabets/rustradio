# Rustradio UI

This is some loose notes about how the UI (mainly TUI) should work. Right now
it's more of a vision than anything else.

## General outline

SDR graph programs should listen on a network port, providing a control
interface. Over this interface it should be possible to:
* View the graph; the blocks and the streams between them.
  * Show CPU use.
  * Show bps / sps through every stream.
* Snoop on any stream.
  * Because it's for a UI, there should be a way to do this without holding up
    the real graph.
  * Exception: Because the network and/or client may not be able to keep up,
    there'll need to be some server code to make sure that *triggered* plots
    don't miss data.
* Give new parameters to blocks. E.g. re-tune the SDR, set the volume, or tune
  symbol sync loop bandwidth.
  * All settings changed should be recorded to disk, so that when optimal
    settings are found, they're not lost.

It should be possible to ad-hoc inspect a graph, as well as creating a fixed
layout of listening points and controls, in some language like JSON.

## Visualizations

* Time/waveform
* Spectrum
* Eye diagram
* Waterfall
* Constellation plots

With triggers and such, inspired by the GNURadio blocks.

## Network protocol

Once there's a standard network protocol, nothing's stopping a QT or Web UI
frontend. But the initial implementation is a TUI.
