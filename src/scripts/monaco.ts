// Monaco, bundled locally (no CDN) and only loaded with the Scripts window.
import * as monaco from "monaco-editor";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import TsWorker from "monaco-editor/language/typescript/ts.worker?worker";
import { loader } from "@monaco-editor/react";

self.MonacoEnvironment = {
  getWorker(_id: string, label: string) {
    return label === "typescript" || label === "javascript" ? new TsWorker() : new EditorWorker();
  },
};

// Types for the `ctx` object, so the editor can autocomplete it.
const CTX_TYPES = `
interface ProxyHeaders { [name: string]: string }
interface RespondWithOptions {
  /** Default 200. */
  status?: number;
  headers?: ProxyHeaders;
  /** A string, or a value that is JSON-encoded. */
  body?: unknown;
  /** Serve a file instead: relative to the scripts folder, or absolute. */
  file?: string;
}
interface BaseContext {
  headers: ProxyHeaders;
  /** The body as text (decoded), or null for binary bodies. */
  body: string | null;
  /** Answer with this response. In onRequest the server is never contacted. */
  respondWith(options: RespondWithOptions): void;
  /** Reads a text file from the scripts folder (or an absolute path); null if missing. */
  readFile(path: string): string | null;
  /** Pauses here until you continue the flow from the breakpoint dialog. */
  breakpoint(): void;
}
interface RequestContext extends BaseContext {
  method: string;
  url: string;
  /** Set to true to refuse the request with 403. */
  blocked: boolean;
}
interface ResponseContext extends BaseContext {
  status: number;
  readonly method: string;
  readonly url: string;
}
declare function onRequest(ctx: RequestContext): void;
declare function onResponse(ctx: ResponseContext): void;
`;

monaco.typescript.javascriptDefaults.setCompilerOptions({
  target: monaco.typescript.ScriptTarget.ES2020,
  allowNonTsExtensions: true,
  checkJs: true,
  noLib: false,
});
monaco.typescript.javascriptDefaults.addExtraLib(CTX_TYPES, "ts:project-proxy/ctx.d.ts");

loader.config({ monaco });

export { monaco };
