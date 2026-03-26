# AI Stream Agent (YouTube) — local-first MVP

Cel: wprowadzić do projektu “prowadzącego AI” (audio / segmenty) jako **narzędzie operatorskie** do:
- opowiadania wiadomości własnym językiem (transformacyjny komentarz),
- prezentowania procesu budowy/testowania bota (rebalancing, analiza danych, strategie),
- docelowo: emisji live przez OBS (sterowane automatycznie).

## Zakres MVP (w repo)

MVP jest **local-first** (bez zewnętrznych płatnych providerów i bez “scrapowania” jako krytycznej ścieżki).

- CLI: `studio-stream-plan`
  - wejście: lokalny JSONL z elementami do narracji (`data/studio/inputs/items.jsonl`)
  - wyjście: JSONL segmentów z szablonem narracji (`data/studio/out/segments.jsonl`)
  - parametry: język (`pl|en`), `style`, `pause_secs`, `limit`

To jest warstwa “redakcyjna” i “ramówkowa” — generuje gotowy tekst do:
- TTS (np. ElevenLabs) albo
- czytania przez człowieka

## Dlaczego tak (kontrakty i koszt)

- Narzędzia postaci/animacji (Animaze/Live2D/UE/Reallusion) nie “sprzedają minut mówienia”.
- Koszt 24/7 to prawie zawsze: **TTS(minuty)** + (opcjonalnie) LLM.
- Ten MVP nie wiąże projektu z żadnym TTS — tylko przygotowuje artefakty.

## Format input JSONL (minimalny)

Każdy wiersz to JSON object. Minimalnie:

```json
{"title":"...", "url":"...", "excerpt":"...", "id":"..."}
```

Rozpoznawane klucze:
- `title` lub `headline` (wymagane)
- `url` lub `link` (opcjonalne)
- `excerpt` / `summary` / `description` (opcjonalne)
- `id` / `source_id` / `guid` (opcjonalne)

## Kolejne kroki (po MVP)

- integracja RSS/API (źródła “stabilne”), cache + deduplikacja
- LLM: generowanie komentarza zamiast listy pytań (z polityką “brand safe”)
- OBS WebSocket: automatyczne przełączanie scen/overlay na podstawie `segments.jsonl`
- osobne persony: host + ekspert + sceptyk (różne style / języki)

## TTS i klonowanie głosu (voice cloning) — kontrakt wejścia/wyjścia

Cel: zorganizować pipeline tak, aby można było:
- czytać segmenty “Twoim głosem” (TTS voice clone),
- łatwo podmieniać dostawcę TTS bez przebudowy reszty (LLM/collector/OBS),
- kontrolować koszty przez liczenie minut mowy.

### Warstwy

1) **Redakcja/ramówka** (już jest w MVP)
- wejście: `items.jsonl`
- wyjście: `segments.jsonl` (z `pause_secs` i `narrator_text`)

2) **TTS adapter** (kontrakt)
- wejście: `StudioSegment` (`narrator_text`, `lang`, `style`, docelowo też `voice_id`)
- wyjście: audio (np. `wav`/`mp3`) + metadane (czas trwania, koszty/minuty)

Rekomendowany artefakt wyjściowy (lokalny, provider-agnostic):
- `data/studio/out/audio/<segment_id>.wav` (albo `.mp3`)
- `data/studio/out/audio/index.jsonl` (mapowanie segment → plik audio → duration)

3) **Emisja (OBS)**
- OBS czyta gotowe pliki audio i odpala je jako źródło (lub przez hotkeye),
- opcjonalnie OBS WebSocket steruje sceną/overlay na podstawie `segments.jsonl`.

### Klonowanie głosu — praktyczne uwagi

- Klonowanie głosu zależy od dostawcy TTS; zwykle wymaga próbek nagrań i potwierdzenia praw do głosu.
- Klonowanie nie usuwa kosztu TTS — dalej płacisz głównie za **minuty wygenerowanej mowy**.
- Dla wielojęzyczności (PL/EN) często trzeba dopracować ustawienia głosu / modelu, żeby akcent był akceptowalny.

