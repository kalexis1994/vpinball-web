import { useCallback, useEffect, useState } from 'react';
import { Content } from './components/Content';
import { Home } from './components/Home';
import { Play } from './components/Play';
import { Player } from './components/Player';
import { Settings } from './components/Settings';
import { listRoms, loadLibrary, storageAvailable } from './lib/library';
import type { Screen, TableEntry } from './lib/types';

/**
 * `/debug` goes straight into the table the dev server serves at
 * `/debug-assets/f14.vpx`, skipping the menu and IndexedDB. It is there to test
 * the renderer without repeating the load on every iteration.
 *
 * Development only. The assets it wants are not in the repository and are not
 * in a build, so on a deployed site the route would load a table that is not
 * there and sit on "loading" for ever; better that it is simply not a route.
 */
function isDebug(): boolean {
  if (!import.meta.env.DEV) return false;
  return window.location.pathname.replace(/\/$/, '') === '/debug';
}

export function App() {
  const [screen, setScreen] = useState<Screen>('home');
  const [playing, setPlaying] = useState<TableEntry | null>(null);
  const [debug, setDebug] = useState(isDebug);

  // What the menu puts under each option. Counted here rather than in `Home`
  // so that adding a table in Content is reflected on the way back out.
  const [tableCount, setTableCount] = useState<number | null>(null);
  const [romCount, setRomCount] = useState(0);

  const recount = useCallback(() => {
    if (!storageAvailable()) {
      setTableCount(0);
      return;
    }
    void loadLibrary().then((t) => setTableCount(t.length));
    void listRoms().then((r) => setRomCount(r.length));
  }, []);

  useEffect(recount, [recount]);

  useEffect(() => {
    const onPop = () => setDebug(isDebug());
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, []);

  const exit = useCallback(() => {
    if (debug) {
      window.history.pushState(null, '', '/');
      setDebug(false);
    }
    setPlaying(null);
    recount();
  }, [debug, recount]);

  if (debug) {
    return (
      <Player
        table={null}
        title="F-14 Tomcat (debug)"
        source={{ kind: 'url', url: '/debug-assets/f14.vpx' }}
        rom={{ status: 'required', name: 'f14_l1', zip: 'f14_l1.zip', alternates: [] }}
        onExit={exit}
      />
    );
  }

  if (playing) {
    return <Player table={playing} source={{ kind: 'library', id: playing.id }} onExit={exit} />;
  }

  const home = () => setScreen('home');

  switch (screen) {
    case 'play':
      return <Play onPlay={setPlaying} onBack={home} onAdd={() => setScreen('content')} />;
    case 'content':
      return <Content onBack={home} onChange={recount} />;
    case 'settings':
      return <Settings onBack={home} />;
    default:
      return <Home tables={tableCount} roms={romCount} onGo={setScreen} />;
  }
}
