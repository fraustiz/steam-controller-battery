# Third-party components

The project itself is under the MIT licence (see `LICENSE`). Two components come
from elsewhere and keep their own terms. Both are compiled into the binary, so
the notices below travel with any distribution of it.

---

## Icon shapes — Material Symbols

The masks in `src/icons.rs` derive from **Material Symbols** by Google,
distributed under the **Apache License, Version 2.0**.

- Upstream: <https://fonts.google.com/icons>
- Licence: <https://www.apache.org/licenses/LICENSE-2.0>

**Modifications.** The original SVGs were split into their sub-paths, recoloured,
combined into charging and dock variants, and rasterised offline into opacity
masks at 16, 20, 24 and 32 pixels. No original file is redistributed; only these
derived masks are.

> Apache 2.0 asks that a copy of the licence accompany the distribution. Dropping
> the text from the link above into `LICENSE-Apache-2.0.txt` closes that point;
> it is not reproduced here rather than risk transcribing it inexactly.

---

## Haptic frequency tables — SteamHapticsSinger

The note table in `src/hid.rs` — the frequencies driving the ringtone, and the
report layout that carries them — derives from **SteamHapticsSinger** by
CrazyCritic89, itself a fork of **SteamControllerSinger** by Pila, by way of the
sibling project [shs-studio](https://github.com/fraustiz/steam-haptics-studio).

That work is distributed under the **BSD 3-Clause License**, reproduced verbatim
below.

- Upstream: <https://github.com/CrazyCritic89/SteamHapticsSinger>

**Modifications.** A MIDI file was converted offline into a frozen event table
using the upstream frequency tables and channel-to-actuator mapping. The tables
themselves are not shipped — only the note values they produced for one melody.

```text
Copyright (c) 2015-2016 Pila
Copyright (c) 2022-2026 Crazy, AAGaming
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

---

## Build dependencies

Neither ends up specially licensed in the binary beyond its own permissive
terms, but for completeness:

| Crate | Licence |
|---|---|
| [`hidapi`](https://crates.io/crates/hidapi) | MIT |
| [`windows-sys`](https://crates.io/crates/windows-sys) | MIT or Apache-2.0 |
