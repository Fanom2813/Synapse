# Platform notes

Platform gaps and workarounds (AGENTS.md §10). `cfg(target_os)` decisions live in `synapse-app/src/platform.rs`.

## macOS: closing the window must not destroy it (Touch Bar Macs)
Destroying a native `NSWindow` on a Mac with a Touch Bar can crash the whole app with `SIGILL`:
AppKit's `_NSTouchBarFinderObservation invalidate` throws `NSRangeException` ("Cannot remove an observer ...
for the key path nextResponder ... not registered") in the next display cycle, and nothing catches it.
Seen on MacBookPro15,1 / macOS 15.7 with v0.83.6. Same crash is reported upstream (longbridge/gpui-kit#3192)
and in other GPUI/winit apps; it is in AppKit/GPUI, not our code.

Workaround: on macOS the close button is intercepted (`Window::on_window_should_close`), the app is hidden
(`App::hide`, i.e. `[NSApp hide:]`) and the same window is reused on the next open. The window is never
destroyed (no destroy-after-60 s timer effect, because the window stays live). Other platforms keep the
hide-then-destroy behaviour from `lifecycle.rs`.
