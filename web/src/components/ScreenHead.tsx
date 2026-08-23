// The bar every screen below the menu carries.
//
// A way back, a title, and room for whatever that screen needs beside it — a
// search box, an add button. Shared so the way out is in the same place on
// every screen, which is most of what makes a menu feel like one thing.

/** The bar every screen below the menu carries: a way back, and a title. */
export function ScreenHead({
  title,
  onBack,
  children,
}: {
  title: string;
  onBack: () => void;
  children?: React.ReactNode;
}) {
  return (
    <header className="screen-head">
      <button className="btn btn-ghost btn-back" onClick={onBack} aria-label="Back to the menu">
        <svg viewBox="0 0 24 24" fill="none" width="18" height="18" aria-hidden="true">
          <path
            d="M15 5l-7 7 7 7"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>
      <h1>{title}</h1>
      {children}
    </header>
  );
}
