export function Index() {
  return (
    <div className="p-6">
      <h1 className="text-xl font-semibold">tldr</h1>
      <p className="text-slate-600 mt-2">
        Launch a session with <code className="px-1 bg-slate-200 rounded">tldr &lt;pr-number&gt;</code> from a git repo.
      </p>
      <p className="text-slate-600 mt-2">
        {/* TODO(future): list active sessions from /api/sessions */}
      </p>
    </div>
  );
}
