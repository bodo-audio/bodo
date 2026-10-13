# Factory content

The sounds and other content that ship with Bodo, so you can start making music right after installing it. The app's browser lists this folder, and projects refer to its files as `factory:<path>`, for example `factory:Packs/Bodo Drums/Kick.wav`.

## Bodo Drums

`Packs/Bodo Drums/` is a drum kit of eleven one-shots: `Kick`, `Snare`, `Clap`, `HiHat`, `Open Hat`, `Rim`, `Low Tom`, `High Tom`, `Crash`, `Shaker` and `808 Bass` (tuned to C2). Each is a 48 kHz mono 24-bit WAV, peak-normalized to -1 dBFS.

Every sound is synthesized from scratch by `cargo xtask factory` (`xtask/src/factory.rs`) out of oscillators, seeded noise, envelopes and filters. Nothing is sampled or copied from another product or library. To change a sound, edit the generator, run `cargo xtask factory` and commit the new files. A run on the same machine always writes the same files; another platform may round a few samples differently in the last bit, so the committed files are the reference.

## License

All factory content in this folder is dedicated to the public domain under [CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/) (full text in [`LICENSE`](LICENSE)). You may use it in your music, commercial or not, and copy, change and share it, without asking and without credit. This applies only to this folder; Bodo's code keeps its own licenses.
