# HØJT Audio plugins

Copyright (c) 2026 Rasmus Højte.

This program is free software: you can redistribute it and/or modify it under the terms of the
GNU General Public License as published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without
even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU
General Public License in [LICENSE](LICENSE) for more details.

## Name and logo

The licence covers the code. It does not give a right to the name HØJT, the name Hojt Audio or the
HØJT logo. A modified version that is passed on must carry another name and another logo, so
nobody takes it for the original.

## Third-party components

The plugins are built with these, each under its own licence:

| Component | Licence |
|-----------|---------|
| [NIH-plug](https://github.com/robbert-vdh/nih-plug) | ISC |
| [vst3-sys](https://github.com/robbert-vdh/vst3-sys) (VST3 bindings) | GPL-3.0 |
| [clap-sys](https://github.com/micahrj/clap-sys) (CLAP bindings) | MIT or Apache-2.0 |
| [egui](https://github.com/emilk/egui) | MIT or Apache-2.0 |
| [baseview](https://github.com/RustAudio/baseview) | MIT or Apache-2.0 |
| [hound](https://github.com/ruuda/hound) | Apache-2.0 |
| Bebas Neue, Bowlby One SC, IBM Plex Mono (fonts) | SIL Open Font License 1.1 |

The font licences are in `crates/suite_common/assets/fonts/` and in every download. The full list
of Rust crates a build contains is in `Cargo.lock`.

VST is a registered trademark of Steinberg Media Technologies GmbH.
