import type { RomInfo } from '../lib/types';

/**
 * The key fact in the menu: whether the table needs a ROM, and which one.
 *
 * The name comes from parsing the table's VBScript, because the VPX format
 * does not keep it in any field. It is a heuristic, not an exact reading.
 */
export function RomBadge({ rom, available }: { rom: RomInfo; available: boolean }) {
  if (rom.status === 'none') {
    return <span className="badge badge-ok">No ROM</span>;
  }

  if (rom.status === 'unknown') {
    return (
      <span className="badge badge-warn" title="The table builds the ROM name at runtime">
        Needs a ROM · name not detected
      </span>
    );
  }

  // Whether the ROM is on hand is the thing a player actually wants to know:
  // without it the table loads and the ball rolls, but nothing ever scores.
  if (!available) {
    return (
      <span className="badge badge-rom" title={`${alternatesHint(rom)}\nDrop the zip on this page to add it.`}>
        ROM <strong>{rom.zip}</strong> · missing
        {rom.alternates.length > 0 && <em> +{rom.alternates.length}</em>}
      </span>
    );
  }

  return (
    <span className="badge badge-ok" title={alternatesHint(rom)}>
      ROM <strong>{rom.zip}</strong> · ready
    </span>
  );
}

function alternatesHint(rom: RomInfo): string {
  if (rom.alternates.length === 0) return `PinMAME set: ${rom.name}`;
  return `PinMAME set: ${rom.name}\nAlternatives in the script: ${rom.alternates.join(', ')}`;
}
