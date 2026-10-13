# ADR 0013: Paid stems and plugins in the marketplace

- **Status:** Accepted
- **Date:** 2026-10-13
- **Deciders:** hasan-muzak
- **Supersedes:** the parts of ADR 0010 that keep the marketplace free and open-source only

## Context

ADR 0010 made the marketplace free: no payments, open-source plugins only, and plugins built from source by Bodo CI. The owner's marketplace sketch (2026-10-13) changes the product:

- items carry prices ($4 stems, a $19 plugin) next to free ones;
- plugins can be bought, and their cards show either a license or a price;
- a Library section lists owned, installed and updated items;
- a Sell section lets users upload stems and publish plugins.

The owner also plans a bridge that lets people build their own plugins with AI agents (project.md section 21.2), and selling them is a natural next step. Bodo Kit is MIT OR Apache-2.0, so plugins built with it may be sold and kept closed source (ADR 0002). The DAW itself stays free, and the Bodo Cloud subscription stays its main revenue (ADR 0008).

## Decision

- **Free and paid items side by side.** Anyone approved as a publisher can list free items. Sellers can also price stems, sample packs, presets and plugins. Bodo's own stock content stays free.
- **Sellers** verify their identity and payout account through Stripe Connect before they can sell. Buyers pay through Stripe Checkout, which opens in the system browser, so the app never handles card details. Purchases belong to the Bodo account, the same account used for Bodo Cloud.
- **Licenses:**
  - free sounds: CC0-1.0 or CC-BY-4.0, as before;
  - paid sounds: a Bodo content license that lets the buyer use the sounds in their own music, commercial releases included, but not resell or redistribute them as sounds;
  - open-source plugins: an OSI license, free or paid;
  - closed-source plugins: any license the seller chooses, shown on the item page.
- **Plugin builds:**
  - Open-source plugins are still built from source by Bodo CI, as in ADR 0010.
  - Closed-source plugins arrive as binaries for each platform, signed by their seller: notarized on macOS, Authenticode-signed on Windows. Bodo CI runs the validators (clap-validator, pluginval, auval) and a malware scan, and rejects anything that fails.
  - Every third-party plugin runs in the shared sandbox bridge by default (ADR 0004), so a faulty plugin can't take Bodo down.
- **Delivery:** paid items download through short-lived links tied to the buyer's account. Everything else in ADR 0010 still applies: the publisher signature, the index countersignature, hash checks, previews before download, and curator approval for every new seller and every first version.
- **No copy protection from Bodo.** Bodo doesn't add DRM to items. Plugin sellers may add their own license checks.
- **The assistant never spends money.** It can search the marketplace, preview items and fetch free ones after the user confirms. A purchase always starts with the user pressing Buy.
- **Still open, and business decisions for the owner:** Bodo's commission on sales, who handles sales tax and VAT (Bodo with Stripe Tax, or a merchant-of-record service), the refund policy, the seller payout terms, and a legal review of the content license text.

## Consequences

- Creators, including people who build plugins with AI agents, can earn money in Bodo, which brings content and plugins to the platform.
- Selling digital goods worldwide brings tax, refund, chargeback and fraud handling. These have to be settled before the marketplace launches in M8.
- Closed-source binaries can't be checked the way source builds can. Seller verification, signing, scanning and sandboxing reduce the risk without removing it.
- The marketplace backend gains payments, payouts and entitlements, and its threat model grows accordingly.

## Alternatives considered

- **Keep the marketplace free (ADR 0010):** simpler and safer, but it rules out the creator economy the owner wants.
- **Accept only open-source plugins, even paid ones:** few commercial plugin makers would join.
- **In-app card payments:** they would put card data in the app and add compliance work. Stripe Checkout in the browser avoids both.
