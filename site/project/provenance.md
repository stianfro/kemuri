# Source provenance

Kemuri is an independent implementation inspired by
[SmokePing](https://oss.oetiker.ch/smokeping/). SmokePing is copyright Tobias
Oetiker and other SmokePing contributors. SmokePing is available under the GNU
General Public License, version 2 or later.

Kemuri does not contain SmokePing source code, documentation, templates,
styles, images, or other assets. Kemuri does not link to SmokePing libraries.
The probe system, storage system, web UI, and graph renderer in Kemuri have
independent implementations.

Kemuri uses the MIT License. Do not copy SmokePing code or assets into Kemuri.
A contributor who proposes third-party code or an asset must:

1. Identify its source and copyright owner.
2. Confirm that its license is compatible with the MIT License.
3. Keep all notices that its license requires.
4. Record the source and license in
   [`PROVENANCE.md`](https://github.com/stianfro/kemuri/blob/main/PROVENANCE.md)
   when the material becomes part of Kemuri.

Kemuri dependencies keep their own licenses. The Rust dependency license check
uses `cargo deny`. The production web dependency check uses `npm audit`.
