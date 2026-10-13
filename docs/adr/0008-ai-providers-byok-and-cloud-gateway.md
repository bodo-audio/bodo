# ADR 0008: AI providers, BYOK, local models and the Bodo Cloud gateway

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak

## Context

The business model is that everything is free except an AI subscription. Users must be able to run the full assistant with their own keys (OpenRouter, OpenAI, Anthropic, Gemini) or fully offline with local models (Ollama, LM Studio, llama.cpp). Bodo is GPL, so any feature placed behind a client-side paywall would simply be stripped out of forks.

## Decision

- **The agent always runs locally** inside Bodo (`bodo-agent`). The subscription buys model access, not features.
- **`bodo-llm`** defines an `LlmProvider` trait (`caps`, `stream`) with three implementations:
  - `OpenAiCompatible { base_url, key }`, which covers OpenAI, OpenRouter, Ollama, LM Studio, llama.cpp, vLLM and Bodo Cloud;
  - `AnthropicMessages`;
  - `GeminiNative`, which also handles audio input.

  `genai` may back the adapters, but the trait belongs to us.
- **Keys** are kept in the OS keychain, never in projects or logs.
- **Models without native tool calling** fall back to JSON-schema-constrained decoding.
- **Cost:** a cost meter with caps per session and per day.
- **Bodo Cloud** is an OpenAI-compatible gateway: auth, routing to upstream providers, metering, credits, caps and zero data retention. The client points `OpenAiCompatible` at the Cloud base URL with a session token from Hub or DAW sign-in.
- **Privacy:**
  - only the chosen provider receives data;
  - audio is uploaded only with per-project opt-in;
  - a "Data sent" inspector shows every request;
  - telemetry is off by default.
- **Untrusted content** (marketplace metadata, preset names, MIDI text, plugin profiles) is passed to models only inside typed, length-capped `untrusted_content` envelopes.

## Consequences

- BYOK users, local-model users and subscribers get identical assistant features. Subscriptions sell convenience: no key setup, credits and curated model routing.
- No feature is hidden behind a paywall a fork could remove, so the business model holds up under the GPL.
- We have to support capability differences between models, through tool profiles and constrained decoding.

## Alternatives considered

- **Server-side agent for subscribers:** it would split features and raise privacy concerns.
- **Agent locked behind the subscription:** it could be removed from a GPL fork, and it contradicts the "free" promise.
- **One provider only:** it excludes local and offline users.
