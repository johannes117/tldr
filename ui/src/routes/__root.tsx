import { Outlet, Link } from "@tanstack/react-router";

export function Root() {
  return (
    <div className="min-h-screen flex flex-col">
      <header className="border-b bg-white px-4 py-2 flex items-center gap-4">
        <Link to="/" className="font-bold">tldr</Link>
        <span className="text-slate-500 text-sm">local PR review</span>
      </header>
      <main className="flex-1 min-h-0">
        <Outlet />
      </main>
    </div>
  );
}
