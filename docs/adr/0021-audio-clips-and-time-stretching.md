# ADR 0021: Audio clips and time stretching

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

M3 puts audio on the playlist ([project.md section 10.3](../../project.md#103-playlist)): any track holds audio clips, which start on the right frame, can be trimmed, slipped and faded, and follow the project tempo by resampling or by time stretching. The audio thread may not allocate, lock or keep state that seeking would have to repair ([AGENTS.md section 4](../../AGENTS.md)). Good time stretching is a research field of its own, and the DAW crates may use GPL-compatible code.

## Decision

- **Audio clips are channels.** `ChannelKind::AudioClip` holds the sample, the stretch mode (`none`, `resample` or `stretch`) and the tempo the audio was made at. Clips on the playlist point at the channel, so one file played many times is one channel. `channel.add_audio`, `clip.place_audio`, `clip.place_sound`, `clip.set_stretch` and `clip.set_fades` edit them.
- **Every block describes itself.** When the song index is built, an `AudioIndex` sorts the audio clips. Each block, a binary search finds the clips the block overlaps and pushes an `AudioSegment` per clip into `BlockContext.audio`: the block frame, frame count, read position, step, stretch key and the clip position for fades. `AudioClipNode` keeps no playback position: it reads the segments with Hermite interpolation (exact at whole frames), applies the fades, then the channel's volume, pan and mute, ramped like the sampler. Seeking, stopping and looping need no special handling.
- **Stretching uses Signalsmith Stretch** (`signalsmith-stretch`, MIT, C++ built with `cc` and `bindgen`). The control thread renders a stretched copy per sample and ratio into a `StretchCache`, and the audio thread reads the copy like any sample. Until a copy is ready, the clip plays resampled, so it is never silent or late.

## Consequences

- Audio clips are sample-accurate at any block size and with any number of workers, and the RT tests cover a moved clip without allocation.
- Building needs a C++ compiler and libclang on every platform; CI runners have both.
- The first use of a new ratio pauses the control thread while the copy renders; moving that to a background job is follow-up work.
- A tempo change inside a clip uses the tempo at the clip's start; files longer than 30 seconds still load into memory until the clip node reads through the disk streamer of [ADR 0016](0016-disk-streaming-in-kit-audio.md).

## Alternatives considered

- **Rubber Band:** excellent quality, but GPL-2.0-or-later with a commercial dual license, a larger C++ build, and FFTW or its own FFT to choose between.
- **Our own WSOLA or phase vocoder:** months of tuning to reach the quality users expect.
- **Stretching on the audio thread:** costs CPU and latency on every block, and keeps allocating state that a seek would have to rebuild.
