// The first thing you see.
//
// Three doors and nothing else. A pinball cabinet has no file manager on the
// front of it, and the three things a player might want on opening this are far
// enough apart to be worth separating: put a ball in play, get more tables in,
// or change how it behaves.
//
// The counts under each one are the point of putting them here rather than in a
// nav bar. "Play" with nothing behind it is a dead end, and a player who is
// told so on the first screen goes to Content instead of finding an empty list.

import type { Screen } from '../lib/types';

interface Props {
  /** How many tables are in the library, or null while it is still loading. */
  tables: number | null;
  /** How many ROM sets are stored. */
  roms: number;
  onGo: (screen: Screen) => void;
}

export function Home({ tables, roms, onGo }: Props) {
  const empty = tables === 0;

  return (
    <main className="home">
      <header className="home-head">
        <h1 className="home-title">vpinball&#8203;-web</h1>
        <p className="home-sub">Visual Pinball, in the browser</p>
      </header>

      <nav className="home-menu">
        <MenuItem
          label="Play"
          hint={
            tables === null
              ? 'reading the library…'
              : empty
                ? 'no tables yet — start with Content'
                : `${tables} ${tables === 1 ? 'table' : 'tables'} ready`
          }
          icon={<PlayIcon />}
          disabled={tables === null || empty}
          onClick={() => onGo('play')}
        />
        <MenuItem
          label="Content"
          hint={
            roms > 0
              ? `${roms} ROM ${roms === 1 ? 'set' : 'sets'} stored`
              : 'add tables and ROMs'
          }
          icon={<ContentIcon />}
          onClick={() => onGo('content')}
        />
        <MenuItem
          label="Settings"
          hint="sound and view"
          icon={<SettingsIcon />}
          onClick={() => onGo('settings')}
        />
      </nav>
    </main>
  );
}

function MenuItem({
  label,
  hint,
  icon,
  disabled,
  onClick,
}: {
  label: string;
  hint: string;
  icon: React.ReactNode;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button className="home-item" onClick={onClick} disabled={disabled}>
      <span className="home-item-icon" aria-hidden="true">
        {icon}
      </span>
      <span className="home-item-text">
        <span className="home-item-label">{label}</span>
        <span className="home-item-hint">{hint}</span>
      </span>
      <span className="home-item-chevron" aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none">
          <path d="M9 5l7 7-7 7" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </span>
    </button>
  );
}

// --- Icons -------------------------------------------------------------------
//
// Drawn rather than fetched: three shapes cost less than a request, and they
// stay sharp on a phone.

/** A ball in a shooter lane, which is what Play means here. */
function PlayIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" role="presentation">
      <rect x="14" y="2" width="7" height="20" rx="3.5" stroke="currentColor" strokeWidth="1.6" opacity="0.5" />
      <circle cx="17.5" cy="17" r="2.6" fill="currentColor" />
      <path d="M9 20V7a4 4 0 0 1 4-4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" opacity="0.5" />
      <path d="M3 15l5-3v6l-5-3z" fill="currentColor" />
    </svg>
  );
}

/** A file going into a box. */
function ContentIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" role="presentation">
      <path d="M3 13v6a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-6" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      <path d="M12 3v11" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      <path d="M8 10l4 4 4-4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/** Sliders, because settings here are levels rather than switches. */
function SettingsIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" role="presentation">
      <path d="M4 7h16M4 17h16" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" opacity="0.5" />
      <circle cx="9" cy="7" r="2.6" fill="var(--bg)" stroke="currentColor" strokeWidth="1.6" />
      <circle cx="16" cy="17" r="2.6" fill="var(--bg)" stroke="currentColor" strokeWidth="1.6" />
    </svg>
  );
}
