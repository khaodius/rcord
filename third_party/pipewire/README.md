# PipeWire 1.0.5 headers

The public C headers of [PipeWire](https://gitlab.freedesktop.org/pipewire/pipewire) 1.0.5
(`pipewire-0.3` and `spa-0.2`), unchanged, under the MIT license in [COPYING](COPYING).

rcord's Linux packages are built on Ubuntu 22.04 so they run on older distributions. That
release ships PipeWire 0.3.48, whose headers are too old for the `pipewire` crate's generated
bindings. CI and the release workflow point `pkg-config` at `pkgconfig/` here, so the bindings
are generated from these headers while rcord still links against the system's
`libpipewire-0.3.so.0`, whose ABI is stable across 0.3 and 1.x. Nothing here is compiled into
rcord except the bindings.

On a current distribution with PipeWire 1.0 or later installed, the system headers work and
this folder is not needed.
