// A page that outlived the files it was built from.
//
// The site is static and every asset is named after a hash of what is in it,
// so a deployment writes new names and takes the old ones away. A browser
// holding the previous page in its cache asks for names nobody serves any
// more, and the first thing that needs one fails: the wasm module the editor
// is, or the rule database it fetches to identify a file. What the reader sees
// is a file that will not open, and trying again does not help, because the
// page doing the asking is the stale one.
//
// So the page asks for itself again, once. A reload revalidates the document,
// which is the one request that can find out there is a newer page; every
// asset it then names is one that exists.
//
// Once, and no more. A deployment really is broken, or a browser really has no
// WebAssembly, and reloading either of those forever is worse than saying so:
// a loop takes the tab away from the reader, and they cannot read the message
// telling them what went wrong. So the attempt is remembered for the tab, and
// the second failure is reported rather than answered.

/** That this tab has already reloaded for a stale asset, so it must not do it
 *  again. Session storage rather than local: a new tab is a new chance, and a
 *  reader who comes back tomorrow should not be held to what happened today.
 *  Reading it can throw where site data is blocked, and a browser that will
 *  not remember gets the one reload rather than none. */
const TRIED = "qubero.reloaded-for-assets";

function alreadyTried(): boolean {
  try {
    return sessionStorage.getItem(TRIED) !== null;
  } catch {
    return false;
  }
}

function remember(): void {
  try {
    sessionStorage.setItem(TRIED, String(Date.now()));
  } catch {
    // A browser that will not remember still gets the reload. The worst that
    // follows is a second one, and the module cache in `doc.ts` means the
    // failure has to happen again for that to be reached at all.
  }
}

/**
 * Ask for the page again, unless this tab already has. True when a reload is
 * on its way and the caller should stop what it was doing; false when the
 * reload has been spent and the failure is the caller's to report.
 */
export function reloadForStaleAssets(): boolean {
  if (alreadyTried()) return false;
  remember();
  location.reload();
  return true;
}

/**
 * Watch for a chunk the page cannot fetch.
 *
 * Vite raises `vite:preloadError` when a dynamically imported chunk fails to
 * load, which is exactly this case and arrives before the import rejects. The
 * event is cancelled so that Vite does not also rethrow into a page that is
 * already on its way out.
 *
 * `spare` says whether the page is the reader's to take: not every chunk is
 * the editor, and the rule database is fetched long after a file is open. A
 * reload there would answer a failed identification by throwing away the file
 * the reader was reading, and their edits with it. Nothing is worth that, so a
 * page with something open keeps what it has and does without.
 *
 * Only in a build: the dev server has no hashed names and nothing to go stale,
 * and the event is never raised there.
 */
export function watchForStaleAssets(spare: () => boolean): void {
  window.addEventListener("vite:preloadError", (event) => {
    if (spare() && reloadForStaleAssets()) event.preventDefault();
  });
}
