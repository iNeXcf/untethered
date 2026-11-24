Based on the MIT-licensed `smithay-clipboard` 0.7.3 crates.io source.
The only functional changes are the `FileDrag` hook in `lib.rs` and the
`DataDeviceHandler` drag callbacks in `state.rs`. The Iced clipboard already
owns a Wayland data device for the window, so use that device to negotiate
copy-only URI lists rather than creating a second seat/competing drag target.
Remove the patch when Iced/winit supports native Wayland file drops.
