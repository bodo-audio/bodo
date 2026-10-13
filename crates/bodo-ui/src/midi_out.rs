//! Sends MIDI Out channels' notes to their ports (project.md section 10.2). A thread drains the engine's queues every
//! millisecond and sends each message to its port, opening ports the first time they are needed; a port that can't
//! be opened is tried again a second later. The audio thread never touches a MIDI driver.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bodo_kit_device::midi::MidiOutput;
use bodo_mix::midi_out::MidiOutTap;

/// How often the thread looks for messages.
const POLL: Duration = Duration::from_millis(1);
/// How long a port that failed to open is left alone.
const RETRY: Duration = Duration::from_secs(1);

/// The thread that sends MIDI Out messages; dropping it stops the thread.
#[derive(Debug)]
pub(crate) struct MidiOutSender {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MidiOutSender {
    /// Starts sending what `tap` collects.
    pub(crate) fn start(tap: MidiOutTap) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name(String::from("bodo-midi-out"))
            .spawn(move || {
                let mut ports: HashMap<String, Result<MidiOutput, Instant>> = HashMap::new();
                while !flag.load(Ordering::Relaxed) {
                    tap.drain(|port, message| {
                        let entry = ports.entry(port.to_owned()).or_insert_with(|| MidiOutput::open(port).map_err(|_| Instant::now()));
                        if let Err(failed) = entry
                            && failed.elapsed() > RETRY
                        {
                            *entry = MidiOutput::open(port).map_err(|_| Instant::now());
                        }
                        if let Ok(output) = entry
                            && let Err(error) = output.send(message.channel, message.message)
                        {
                            tracing::warn!(%error, port, "a MIDI message could not be sent");
                        }
                    });
                    std::thread::sleep(POLL);
                }
            })
            .map_err(|error| tracing::warn!(%error, "MIDI Out channels can't send: no thread"))
            .ok();
        Self { stop, thread }
    }
}

impl Drop for MidiOutSender {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
