# Meteora DLMM — Anchor swap `Program data:`

## Oficjalna nazwa eventu w IDL

W `MeteoraAg/dlmm-sdk` → `idls/dlmm.json` sekcja **`events`**: wpis **`"name": "Swap"`** z dyskryminatorem:

```json
{ "name": "Swap", "discriminator": [81, 108, 227, 190, 205, 208, 10, 196] }
```

To odpowiada pierwszym 8 bajtom `sha256("event:Swap")` (konwencja Anchor).

**Nie** używać `event:SwapEvent` (`[64,198,205,232,38,8,113,226]`) do parsowania Meteory — ten sam hash co „Raydium-style” `SwapEvent` może pojawić się w **innych** programach w tej samej transakcji (np. Jupiter); payload nie jest layoutem Meteory.

## Kod

- Stała: `METEORA_SWAP_EVENT_DISCRIMINATOR` w `crates/protocols/src/events/meteora_swap_event.rs`
- Parser: `parse_meteora_swap_event_for_pool` — tylko powyższy dyskryminator + Borsh `MeteoraDlmmSwapEvent`

## Weryfikacja na mainnecie (2026-03)

- Skrypt `scripts/scan-meteora-program-data-discriminators.mjs` — zbiera unikalne pierwsze 8 bajtów z `Program data:` w transakcjach z `getSignaturesForAddress(LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo)` (próbka zależy od RPC i momentu).
- Skrypt `scripts/fetch-meteora-swap-event-fixture.mjs` — szuka pierwszego `Program data:` z pełnym payloadem swapu (≥ 137 bajtów) i dyskryminatorem IDL `Swap` (paginacja `before=`).

## Fallback

Przy agregatorach część transakcji ma tylko `Program data:` z **innych** programów; wtedy `swap_sync` używa heurystyk (vaulty / partial) — patrz `crates/cli/src/swap_sync.rs`.

## Kierunek (`swap_for_y`)

- Snapshot: `token_mint_a` = X, `token_mint_b` = Y.
- `swap_for_y == true` → X → Y → `a_to_b`; `false` → `b_to_a`.
