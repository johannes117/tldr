// Shared fixture JSON for mocked backend responses.
import type { Page, Route } from "@playwright/test";

export const PR = {
  pr: {
    number: 1,
    title: "Fix: improve diff rendering",
    body: "## Summary\n\nMakes the diff faster.\n\n## src/a.ts\nrefactor",
    state: "open",
    head_sha: "aaa111",
    base_sha: "bbb222",
    head_ref: "feature",
    base_ref: "main",
    node_id: "NODE",
    author: "octocat",
    html_url: "https://github.com/o/r/pull/1",
  },
  slug: "o/r",
  worktree: "/tmp/tldr-worktree-1",
};

export const DIFF = {
  files: [
    {
      path: "src/a.ts",
      old_path: "src/a.ts",
      status: "modified",
      hunks: [
        {
          header: "@@ -1,3 +1,4 @@",
          old_start: 1,
          new_start: 1,
          lines: [
            { kind: "ctx", content: "one", old_line: 1, new_line: 1 },
            { kind: "del", content: "two", old_line: 2, new_line: null },
            { kind: "add", content: "two changed", old_line: null, new_line: 2 },
            { kind: "add", content: "new", old_line: null, new_line: 3 },
            { kind: "ctx", content: "three", old_line: 3, new_line: 4 },
          ],
        },
      ],
      is_generated: false,
      is_large: false,
      is_binary: false,
      is_image: false,
      stats: { added: 2, removed: 1 },
    },
    {
      path: "src/b.ts",
      old_path: null,
      status: "added",
      hunks: [
        {
          header: "@@ -0,0 +1,2 @@",
          old_start: 0,
          new_start: 1,
          lines: [
            { kind: "add", content: "hello", old_line: null, new_line: 1 },
            { kind: "add", content: "world", old_line: null, new_line: 2 },
          ],
        },
      ],
      is_generated: false,
      is_large: false,
      is_binary: false,
      is_image: false,
      stats: { added: 2, removed: 0 },
    },
  ],
};

export const EMPTY_DRAFT = {
  pr: 1,
  body: "",
  verdict: null,
  comments: [],
  file_state: {},
  updated_at: "2025-01-01T00:00:00Z",
};

export const FRAMING = {
  reviewer: { login: "alice", email: null },
  files: [
    { path: "src/a.ts", owned: true, touched_before: true, expertise_score: 0.8 },
    { path: "src/b.ts", owned: false, touched_before: false, expertise_score: 0.0 },
  ],
};

export const COVERAGE = {
  files: {},
  summary: { new_uncovered_lines: 0, files_with_uncovered: 0 },
  source: null as null,
};

/**
 * Installs route handlers for all /api/* endpoints using in-memory draft state.
 * Returns an object exposing the current draft for assertions.
 */
export function installMocks(page: Page) {
  const state: { draft: any } = { draft: JSON.parse(JSON.stringify(EMPTY_DRAFT)) };

  const json = (route: Route, body: unknown, status = 200) =>
    route.fulfill({
      status,
      contentType: "application/json",
      body: JSON.stringify(body),
    });

  // Register fallback FIRST so specific routes (registered later) take precedence.
  page.route(/\/api\//, (route) => json(route, {}, 200));

  page.route(/\/api\/pr\/\d+$/, (route) => json(route, PR));
  page.route(/\/api\/pr\/\d+\/diff$/, (route) => json(route, DIFF));
  page.route(/\/api\/pr\/\d+\/framing$/, (route) => json(route, FRAMING));
  page.route(/\/api\/pr\/\d+\/coverage$/, (route) => json(route, COVERAGE));
  page.route(/\/api\/pr\/\d+\/collaborators$/, (route) => json(route, { logins: ["alice", "bob"] }));

  page.route(/\/api\/pr\/\d+\/draft$/, (route) => {
    const req = route.request();
    if (req.method() === "GET") return json(route, state.draft);
    if (req.method() === "PUT") {
      const body = JSON.parse(req.postData() || "{}");
      state.draft = { ...state.draft, ...body, updated_at: new Date().toISOString() };
      return json(route, state.draft);
    }
    return route.continue();
  });

  page.route(/\/api\/pr\/\d+\/comments$/, (route) => {
    const req = route.request();
    if (req.method() === "POST") {
      const body = JSON.parse(req.postData() || "{}");
      state.draft.comments = [
        ...(state.draft.comments || []),
        { id: `c${state.draft.comments.length + 1}`, side: "RIGHT", created_at: new Date().toISOString(), ...body },
      ];
      return json(route, state.draft);
    }
    return route.continue();
  });

  page.route(/\/api\/pr\/\d+\/file-state$/, (route) => {
    const req = route.request();
    if (req.method() === "PUT") {
      const body = JSON.parse(req.postData() || "{}");
      const path = body.path || "";
      const cur = state.draft.file_state[path] || { viewed: false, collapsed: false };
      state.draft.file_state[path] = { ...cur, viewed: body.viewed ?? cur.viewed, collapsed: body.collapsed ?? cur.collapsed };
      return json(route, state.draft);
    }
    return route.continue();
  });

  page.route(/\/api\/pr\/\d+\/submit$/, (route) => json(route, { ok: true, result: { posted: true } }));

  page.route(/\/api\/session$/, (route) =>
    json(route, { pid: 0, port: 0, started_at: "x", active_prs: [1], csrf_token_fingerprint: "fp", slug: "o/r" }),
  );

  return state;
}
