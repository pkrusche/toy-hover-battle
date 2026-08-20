# Asset licensing and provenance

All assets in this directory, including the generated sprite sheets, icons,
shader, screenshot, and WAV files, are part of Toy Hover Battle and are
licensed under the repository's [Creative Commons Attribution-NonCommercial
4.0 International license](../LICENSE).

The WAV files in `sfx/` were generated from the prompts and seeds in
`../asset-generation/sfx/prompts.json` using Stability AI's Stable Audio Open
1.0 model.

One file is kept on purpose without being part of the build:

- `sfx/rocket_warning.wav` — generated alongside the other effects and reserved
  for an incoming-rocket alarm; no code path plays it yet, so it is not
  `include_bytes!`-ed into the binary.

Stable Audio Open 1.0, its code, and its model weights are not bundled in this
repository. They are separate Stability AI materials governed by the
[Stability AI Community License](https://huggingface.co/stabilityai/stable-audio-open-1.0/blob/main/LICENSE.md),
not by this repository's CC BY-NC 4.0 license.
