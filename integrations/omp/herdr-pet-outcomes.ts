import { createHash } from "node:crypto";
import { Buffer } from "node:buffer";
import net, { type Socket } from "node:net";
import path from "node:path";

const HERDR_ENV = process.env.HERDR_ENV;
const socketPath = process.env.HERDR_SOCKET_PATH;
const socketEndpoint =
  process.platform === "win32" && socketPath ? `\\\\.\\pipe\\${socketPath}` : socketPath;
const paneId = process.env.HERDR_PANE_ID;
const SOURCE = "herdr-desktop-pet:omp";
const OUTCOME_AUTHORITY = "omp:v1";
const MAX_QUEUED_REPORTS = 8;
const MAX_RESPONSE_BYTES = 16 * 1024;

type SessionManager = {
  getSessionFile?: () => unknown;
  getSessionId?: () => unknown;
};

type ExtensionContext = {
  hasUI?: boolean;
  mode?: string;
  sessionManager?: SessionManager;
  isIdle?: () => boolean;
};

type ExtensionApi = {
  on: (event: string, handler: (event: unknown, ctx: ExtensionContext) => void) => void;
};

type OutcomeName = "running" | "succeeded" | "failed" | "cancelled";

type ActiveTurn = {
  generation: number;
  session: string;
  turn: string;
};

type MetadataRequest = {
  id: string;
  method: "pane.report_metadata";
  params: {
    pane_id: string;
    source: string;
    tokens: Record<string, string>;
  };
};

function enabled() {
  return HERDR_ENV === "1" && !!socketPath && !!paneId;
}

function isExtensionApi(value: unknown): value is ExtensionApi {
  return (
    typeof value === "object" &&
    value !== null &&
    "on" in value &&
    typeof value.on === "function"
  );
}

function isRootUiContext(ctx: unknown): ctx is ExtensionContext {
  if (
    typeof ctx !== "object" ||
    ctx === null ||
    !("hasUI" in ctx) ||
    !("mode" in ctx)
  ) {
    return false;
  }
  return ctx.hasUI === true && ctx.mode === "tui";
}

export function isAbsoluteSessionPath(file: unknown): file is string {
  return (
    typeof file === "string" &&
    (path.posix.isAbsolute(file) || path.win32.isAbsolute(file))
  );
}

function sessionIdentity(ctx: ExtensionContext): string | undefined {
  let sessionFile: unknown;
  try {
    sessionFile = ctx.sessionManager?.getSessionFile?.();
  } catch {
    sessionFile = undefined;
  }
  if (isAbsoluteSessionPath(sessionFile)) {
    return `path:${sessionFile}`;
  }

  let sessionId: unknown;
  try {
    sessionId = ctx.sessionManager?.getSessionId?.();
  } catch {
    sessionId = undefined;
  }
  return typeof sessionId === "string" && sessionId.length > 0
    ? `id:${sessionId}`
    : undefined;
}

function hashSession(identity: string): string {
  return createHash("sha256").update(identity, "utf8").digest("hex");
}

let turnSequence = BigInt(Date.now()) * 1000n;
let requestSequence = BigInt(Date.now()) * 1000n;

function nextTurnToken(): string {
  const wallClock = BigInt(Date.now()) * 1000n;
  if (wallClock > turnSequence) {
    turnSequence = wallClock;
  }
  turnSequence += 1n;
  return turnSequence.toString();
}

function nextRequestId(): string {
  const wallClock = BigInt(Date.now()) * 1000n;
  if (wallClock > requestSequence) {
    requestSequence = wallClock;
  }
  requestSequence += 1n;
  return `${SOURCE}:${requestSequence.toString()}`;
}

function responseVerdict(line: string, expectedId: string): boolean | undefined {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return undefined;
  }
  if (
    typeof value !== "object" ||
    value === null ||
    !("id" in value) ||
    value.id !== expectedId
  ) {
    return undefined;
  }
  if ("error" in value || !("result" in value)) {
    return false;
  }
  const result = value.result;
  return (
    typeof result === "object" &&
    result !== null &&
    "type" in result &&
    result.type === "ok"
  );
}

type ResponseBuffer = {
  value: string;
};

function consumeResponseChunk(
  response: ResponseBuffer,
  expectedId: string,
  chunk: Buffer | string,
): boolean | undefined {
  const chunkBytes =
    typeof chunk === "string" ? Buffer.byteLength(chunk, "utf8") : chunk.byteLength;
  if (Buffer.byteLength(response.value, "utf8") + chunkBytes > MAX_RESPONSE_BYTES) {
    return false;
  }
  response.value += typeof chunk === "string" ? chunk : chunk.toString("utf8");

  for (;;) {
    const newline = response.value.indexOf("\n");
    if (newline < 0) {
      return undefined;
    }
    const line = response.value.slice(0, newline).trim();
    response.value = response.value.slice(newline + 1);
    if (!line) {
      continue;
    }
    const verdict = responseVerdict(line, expectedId);
    if (verdict !== undefined) {
      return verdict;
    }
  }
}

function sendRequestAttempt(request: MetadataRequest, timeoutMs: number): Promise<boolean> {
  if (!enabled()) {
    return Promise.resolve(false);
  }

  const { promise, resolve } = Promise.withResolvers<boolean>();
  let done = false;
  let timeout: NodeJS.Timeout | undefined;
  let socket: Socket | undefined;
  const response: ResponseBuffer = { value: "" };
  const finish = (delivered: boolean) => {
    if (done) return;
    done = true;
    clearTimeout(timeout);
    socket?.destroy();
    resolve(delivered);
  };

  try {
    socket = net.createConnection(socketEndpoint!);
    socket.on("error", () => finish(false));
    socket.on("connect", () => {
      try {
        socket?.write(`${JSON.stringify(request)}\n`);
      } catch {
        finish(false);
      }
    });
    socket.on("data", (chunk: Buffer) => {
      const verdict = consumeResponseChunk(response, request.id, chunk);
      if (verdict !== undefined) {
        finish(verdict);
      }
    });
    socket.on("end", () => finish(false));
    timeout = setTimeout(() => finish(false), timeoutMs);
    timeout.unref?.();
  } catch {
    finish(false);
  }
  return promise;
}

async function sendRequestNow(request: MetadataRequest): Promise<void> {
  if (await sendRequestAttempt(request, 500)) {
    return;
  }
  await sendRequestAttempt(request, 1500);
}

type PendingReport = {
  generation: number;
  session: string;
  terminal: boolean;
  request: MetadataRequest;
};

let pendingReports: PendingReport[] = [];
let drainingReports = false;
let sessionGeneration = 0;
let currentSession: string | undefined;
let rootSession = false;
let activeTurn: ActiveTurn | undefined;

function discardPendingReports() {
  pendingReports = [];
}

function isCurrentReport(report: PendingReport): boolean {
  return (
    rootSession &&
    report.generation === sessionGeneration &&
    report.session === currentSession
  );
}

function enqueueReport(report: PendingReport) {
  if (!isCurrentReport(report)) {
    return;
  }

  if (pendingReports.length >= MAX_QUEUED_REPORTS) {
    // Running updates are disposable snapshots; retain terminal reports when
    // possible so a slow Herdr socket cannot silently erase an outcome.
    const runningIndex = pendingReports.findIndex((queued) => !queued.terminal);
    pendingReports.splice(runningIndex >= 0 ? runningIndex : 0, 1);
  }
  pendingReports.push(report);
  void drainReports();
}

async function drainReports() {
  if (drainingReports) {
    return;
  }
  drainingReports = true;
  try {
    while (pendingReports.length > 0) {
      const report = pendingReports.shift()!;
      if (!isCurrentReport(report)) {
        continue;
      }
      await sendRequestNow(report.request);
    }
  } finally {
    drainingReports = false;
    if (pendingReports.length > 0) {
      void drainReports();
    }
  }
}

function activateRootSession(ctx: unknown, forceReset = false): boolean {
  if (!enabled() || !isRootUiContext(ctx)) {
    return false;
  }

  const identity = sessionIdentity(ctx);
  if (!identity) {
    rootSession = false;
    currentSession = undefined;
    activeTurn = undefined;
    sessionGeneration += 1;
    discardPendingReports();
    return false;
  }

  const session = hashSession(identity);
  if (forceReset || !rootSession || currentSession !== session) {
    sessionGeneration += 1;
    activeTurn = undefined;
    discardPendingReports();
  }
  rootSession = true;
  currentSession = session;
  return true;
}

function reportOutcome(outcome: OutcomeName, turn: ActiveTurn) {
  const request: MetadataRequest = {
    id: nextRequestId(),
    method: "pane.report_metadata",
    params: {
      pane_id: paneId!,
      source: SOURCE,
      tokens: {
        pet_outcome_authority: OUTCOME_AUTHORITY,
        pet_outcome_session: turn.session,
        pet_outcome_turn: turn.turn,
        pet_outcome: outcome,
        pet_outcome_at: String(Date.now()),
      },
    },
  };
  enqueueReport({
    generation: turn.generation,
    session: turn.session,
    terminal: outcome !== "running",
    request,
  });
}

function beginTurn() {
  if (!rootSession || !currentSession) {
    return;
  }
  if (
    activeTurn &&
    activeTurn.generation === sessionGeneration &&
    activeTurn.session === currentSession
  ) {
    return;
  }
  const turn: ActiveTurn = {
    generation: sessionGeneration,
    session: currentSession,
    turn: nextTurnToken(),
  };
  activeTurn = turn;
  reportOutcome("running", turn);
}

function lastAssistantStopReason(event: unknown): unknown {
  if (
    typeof event !== "object" ||
    event === null ||
    !("messages" in event) ||
    !Array.isArray(event.messages)
  ) {
    return undefined;
  }
  for (let index = event.messages.length - 1; index >= 0; index -= 1) {
    const message = event.messages[index];
    if (
      typeof message === "object" &&
      message !== null &&
      "role" in message &&
      message.role === "assistant" &&
      "stopReason" in message
    ) {
      return message.stopReason;
    }
  }
  return undefined;
}

function terminalOutcome(stopReason: unknown): Exclude<OutcomeName, "running"> | undefined {
  if (stopReason === "error") {
    return "failed";
  }
  if (stopReason === "aborted") {
    return "cancelled";
  }
  if (stopReason === "stop") {
    return "succeeded";
  }
  return undefined;
}

export default function (pi: unknown) {
  if (!enabled() || !isExtensionApi(pi)) {
    return;
  }

  pi.on("session_start", (_event, ctx) => {
    if (!activateRootSession(ctx, true)) {
      return;
    }
    // An extension reload can occur while OMP is already working and may not
    // emit another agent_start. Recover only from the authoritative lifecycle
    // state, never from screen text or a cached outcome token.
    if (ctx.isIdle?.() === false) {
      beginTurn();
    }
  });

  pi.on("session_switch", (_event, ctx) => {
    if (!activateRootSession(ctx, true)) {
      return;
    }
    if (ctx.isIdle?.() === false) {
      beginTurn();
    }
  });

  pi.on("agent_start", (_event, ctx) => {
    if (!activateRootSession(ctx)) {
      return;
    }
    beginTurn();
  });

  pi.on("agent_end", (event, ctx) => {
    if (!activateRootSession(ctx) || !activeTurn) {
      return;
    }
    if (
      typeof event === "object" &&
      event !== null &&
      "willContinue" in event &&
      event.willContinue === true
    ) {
      // Auto-retry, compaction, and other scheduled continuations are still the
      // same live operation; do not publish a false terminal outcome.
      return;
    }

    const turn = activeTurn;
    activeTurn = undefined;
    const outcome = terminalOutcome(lastAssistantStopReason(event));
    if (!outcome) {
      return;
    }
    reportOutcome(outcome, turn);
  });

  pi.on("session_shutdown", (_event, ctx) => {
    if (!isRootUiContext(ctx)) {
      return;
    }
    rootSession = false;
    currentSession = undefined;
    activeTurn = undefined;
    sessionGeneration += 1;
    discardPendingReports();
  });
}
