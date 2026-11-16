# Dub manual: writing rules

This folder is Dub's public manual, a Mintlify site (`docs.json`, MDX pages).
It is for DJs first. Engineering notes stay in the repo's `docs/` folder and
are not published.

Run `mint instructions` before a Mintlify task; it prints current guidance for
the installed CLI.

## Facts

- Write only what the code, the repo's docs or the maintainer says. Never
  invent a feature, limit, menu item or label.
- UI words are copied from the app's source (`apple/Dub/`), exactly as the app
  prints them: **LOCKED**, **Preferences ▸ Audio**, **↻**.
- Alpha features (behind **Preferences ▸ Alpha**) are marked as such.
- Screenshots come from the real app, not test renders, and live in `images/`.

## Terminology

- "deck", "control vinyl", "needle", "the mixer" (the DJ's hardware mixer).
- The source switch is **INT · TC · THRU**.
- The two surfaces are **Performance** and **Prep**.
- Dub never mixes: no software crossfader, EQ or gain. Don't describe one.

## Style

- Second person, active voice, one idea per sentence.
- Sentence case for headings; bold for UI elements; code formatting for files
  and commands.
- Short pages a DJ can read standing at the booth.

## Checks

```bash
cd site
mint validate
mint broken-links
mint dev        # preview at http://localhost:3000
```
