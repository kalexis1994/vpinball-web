# Visual Pinball's script library

These are Visual Pinball's own `scripts/*.vbs`, copied verbatim from
[vpinball/vpinball](https://github.com/vpinball/vpinball). They are GPL-3.0,
the same licence as this project.

They are here because a table's script is **not self-contained**: it opens by
pulling in `core.vbs` and the library for its machine — `s11.vbs` for a Williams
System 11, `sam.vbs` for a Stern SAM — by name, at run time, through Visual
Pinball's `GetTextFile`. In Visual Pinball those are files in a Scripts folder
next to the tables. There is no folder in a browser, so they are bundled and
handed to the player before any table is loaded (see `web/src/lib/player.ts`).

Without them a table loads and a ball rolls around, and nothing scores.

To refresh them from a checkout of the original:

```bash
cp ../vpinball/scripts/*.vbs web/src/scripts/
```
