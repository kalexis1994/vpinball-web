// Visual Pinball's script library, bundled with the app.
//
// A table's script is not self-contained: it opens by pulling in `core.vbs`
// and the library for its machine — `s11.vbs`, `sam.vbs` — **by name, at run
// time**. In Visual Pinball those are files in a Scripts folder. There is no
// folder here, so they are bundled and handed to whichever wasm instance is
// about to load a table: the player's, and the bake worker's, which boots the
// same game headless to watch the lamps. Without them a table loads and rolls
// a ball around, and nothing scores.
//
// They are GPL-3.0, the same licence as this project, and come from Visual
// Pinball's own `scripts/` directory.

export const LIBRARIES = import.meta.glob('../scripts/*.vbs', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

/** Hands every bundled library to `add`, by file name. */
export function provideLibraries(add: (name: string, text: string) => void): void {
  for (const [path, text] of Object.entries(LIBRARIES)) {
    add(path.slice(path.lastIndexOf('/') + 1), text);
  }
}
