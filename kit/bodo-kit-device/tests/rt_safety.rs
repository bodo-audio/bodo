//! Checks that rendering a device buffer never allocates. Debug builds only.

#![cfg(debug_assertions)]

use assert_no_alloc::{AllocDisabler, assert_no_alloc, reset_violation_count, violation_count};
use bodo_kit_device::{Block, Pacing, VirtualConfig, VirtualDevice};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

#[test]
fn rendering_does_not_allocate() {
    let config = VirtualConfig { buffer_frames: 5000, input_channels: 2, pacing: Pacing::Freewheel, ..VirtualConfig::default() };
    let mut device = VirtualDevice::new(config, |block: &mut Block<'_>| {
        let input = block.input.channel(0).first().copied().unwrap_or(0.0);
        for channel in block.output.channels_mut() {
            channel.fill(input + 0.25);
        }
    });
    reset_violation_count();
    assert_no_alloc(|| {
        for _ in 0..10 {
            std::hint::black_box(device.render());
        }
    });
    assert_eq!(violation_count(), 0);
}
