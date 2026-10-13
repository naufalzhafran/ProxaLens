// Starting points for new scripts (the header is added by the engine).
export interface ScriptTemplate {
  id: string;
  label: string;
  name: string;
  matches: string;
  body: string;
}

export const TEMPLATES: ScriptTemplate[] = [
  {
    id: "blank",
    label: "Blank script",
    name: "New Script",
    matches: "*",
    body: `function onRequest(ctx) {
  // Modify the request before it is sent.
  // ctx.method, ctx.url, ctx.headers, ctx.body, ctx.blocked
}

function onResponse(ctx) {
  // Modify the response before it reaches the app.
  // ctx.status, ctx.headers, ctx.body
}
`,
  },
  {
    id: "header",
    label: "Add a request header",
    name: "Add header",
    matches: "*://api.example.com/*",
    body: `function onRequest(ctx) {
  ctx.headers["Authorization"] = "Bearer my-test-token";
}
`,
  },
  {
    id: "mock",
    label: "Mock a response",
    name: "Mock response",
    matches: "*://api.example.com/v1/users*",
    body: `function onRequest(ctx) {
  // The server is never contacted.
  ctx.respondWith({
    status: 200,
    headers: { "Content-Type": "application/json" },
    body: { users: [{ id: 1, name: "Test User" }] },
  });
  // Or serve a file from the scripts folder:
  // ctx.respondWith({ file: "users.json" });
}
`,
  },
  {
    id: "json",
    label: "Edit a JSON response",
    name: "Edit JSON",
    matches: "*://api.example.com/*",
    body: `function onResponse(ctx) {
  if (!ctx.body) return;
  const data = JSON.parse(ctx.body);
  data.featureFlag = true;
  ctx.body = JSON.stringify(data);
  console.log("patched", ctx.url);
}
`,
  },
  {
    id: "block",
    label: "Block requests",
    name: "Block trackers",
    matches: "*://*.tracker.example/*",
    body: `function onRequest(ctx) {
  ctx.blocked = true;
}
`,
  },
  {
    id: "breakpoint",
    label: "Pause on a condition",
    name: "Conditional breakpoint",
    matches: "*",
    body: `function onResponse(ctx) {
  if (ctx.status >= 500) {
    ctx.breakpoint(); // edit the response in the breakpoint dialog
  }
}
`,
  },
];
