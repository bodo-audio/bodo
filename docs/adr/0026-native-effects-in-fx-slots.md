# ADR 0026: Native effects in FX slots

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak
- **Refines:** ADR 0009 (slots) and ADR 0004 (plugin hosting), for effects linked into Bodo

## Context

The mixer of M4 needs working FX slots before plugin hosting arrives in M5: slots that process, report latency, take a sidechain, mix dry and wet, and can be automated. Agents need to know what each effect's parameters mean. Delay compensation, sidechain alignment and the null tests also need effects with known behavior, including one whose latency can be set.

Constraints:

- Slot changes must reach a running strip without a recompile, an allocation on the audio thread, or a lock.
- A recompile must never re-create an effect (ADR 0015).
- Slot settings must be saved in the project, in a form that stays valid as effects gain parameters.

## Decision

- **An effect is a Rust type behind a small trait** in `bodo-mix` (`fx/mod.rs`). `Effect::process` works in place on a stereo `EffectBlock`, which also carries the sidechain, the current parameter values, the sample rate and the tempo. `Effect::reset` clears its state.
- **Each effect is described once in a static catalog**, as an `EffectInfo`:
  - a stable uid (`bodo.delay`), a name, a description and a category;
  - whether it has a sidechain input, and whether it is hidden;
  - its parameters (`ParamSpec`): a stable integer id, a name, a unit, a range, a default, a kind (linear, logarithmic, integer, toggle or choice) and tags for agents;
  - functions that create an instance, report its latency for a set of parameter values, and bound that latency.
- **The first effects:** Utility (gain, pan, width, polarity, mono), Filter (low, high and band pass), Delay (tempo-synced with feedback, ping-pong and a high cut) and Ducker (sidechain-keyed gain reduction). A hidden Latency Test effect delays by a parameter, for delay compensation tests.
- **In the model,** a slot holds a `PluginRef` with `PluginFormat::Native` and the uid, plus `params: Vec<ParamValue>`, its parameter values by id. A parameter that isn't listed is at its default, so effects can gain parameters without a migration. Values are clamped to the parameter's range when used. Ids are never reused.
- **The running strip gets slot changes through queues.** Each FX stage owns an SPSC queue of `SlotPatch`es (a slot index and the slot's new state) and a garbage queue back.
  - A new effect is created on the control thread, with its delay lines sized for its maximum latency, and sent as a patch. The replaced state comes back through the garbage queue to be dropped off the audio thread.
  - Enable, mix, latency and parameter values are atomics shared with the running slot (`SlotLive`), so a knob turn is a store.
  - An effect stays loaded until its instance id or uid changes, so recompiles and parameter edits never re-create it.
- **Automation** drives native parameters through `ParamTarget::Plugin(instance, param)`, as normalized values that `ParamSpec::from_normalized` maps to the range. It drives slot mix and enable through `ParamTarget::Slot`.
- **Commands follow the namespaces of project.md section 12.2:**
  - `fx.load`, `fx.remove`, `fx.move`, `fx.swap`, `fx.set_enabled`, `fx.set_mix`, `fx.set_sidechain`, `fx.copy_chain` and `fx.paste_chain` edit slots;
  - `plugin.list`, `plugin.describe`, `plugin.get_params` and `plugin.set_param` cover effects and their parameters.

  Parameters are named by id or name, and values are given in the parameter's unit. `fx.set_sidechain` adds a pre-fader sidechain route when there is none, refuses loops with the cycle path, and refuses a source that already routes audio into the insert. A pasted chain gets new instance ids, and keeps a sidechain only where the target has the route.
- **Slots holding other formats** (CLAP, VST3, AU) pass audio through, with no latency, until hosting arrives in M5.

## Consequences

- Delay compensation, sidechain alignment, slot mixes and automation are tested end to end with real processing, before any third-party plugin exists.
- The catalog gives agents everything they need to set a parameter sensibly: unit, range, default and tags. M5 can describe hosted plugins in the same shape.
- A patch needs a box per slot change, allocated on the control thread. The queues hold 32 patches per strip. When one is full, the patches wait on the control side and go out on the next `update` or `flush`, so nothing is lost.
- Effects in the catalog must keep their uids and parameter ids forever. Retiring a parameter means leaving its id unused.

## Alternatives considered

- **Building the first effects as Bodo Kit plugins** behind the M5 hosting layer: right in the long run, but it would hold the M4 mixer back for M5. Native effects keep the same model shape (`PluginRef` with a format), so stock plugins can replace or join them later.
- **Saving parameters as an opaque state blob,** as hosted plugins do: agents and automation couldn't read or set single values, and every change would rewrite the blob.
- **Recompiling the graph for every slot change:** simple, but a parameter drag would rebuild plans many times a second, and the recompile would have to re-create or carefully move effects.
