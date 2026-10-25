/**
 * Electron-shaped first-window surface over the one authenticated Keld app link.
 * The Core registry is the sole owner of ids, native state and close decisions.
 * No renderer/Node grant, alternate transport, or optimistic native state exists.
 */
import { EventEmitter } from "node:events";
import {
  app as hostApp,
  getWindowSnapshot,
  getWindowSnapshots,
  invokeWindow,
  invokeWindowBlocking,
  onWindowFactApplied,
} from "../../api/src/app.ts";
import type {
  WindowEvent,
  WindowRef,
  WindowRequest,
} from "../../api/src/window-codec.ts";
import { emitNativeAppEvent } from "./app.ts";

/** Known draw.io preferences; other fields are reported, never made authoritative. */
export interface WebPreferences {
  preload?: string;
  additionalArguments?: readonly string[];
  webviewTag?: boolean;
  contextIsolation?: boolean;
  nodeIntegration?: boolean;
  nodeIntegrationInWorker?: boolean;
  nodeIntegrationInSubFrames?: boolean;
  webSecurity?: boolean;
  allowRunningInsecureContent?: boolean;
  disableBlinkFeatures?: string;
  spellcheck?: boolean;
  readonly [unknownField: string]: unknown;
}

export interface BrowserWindowOptions {
  width?: number;
  height?: number;
  x?: number;
  y?: number;
  backgroundColor?: string;
  icon?: string;
  webPreferences?: WebPreferences;
  readonly [unknownField: string]: unknown;
}

/** Only the Host's original identity pair can select a window. */
const windows = new Map<number, BrowserWindow>();

function diagnostic(message: string): Error {
  return new Error(
    `KELD-COMPAT-010: ${message}. Use the Keld isolated webview defaults.`,
  );
}

function checkOptions(options: BrowserWindowOptions): void {
  if (
    options === null ||
    typeof options !== "object" ||
    Array.isArray(options)
  ) {
    throw diagnostic("BrowserWindow options must be an object");
  }
  const prefs = options.webPreferences;
  if (prefs === undefined) return;
  if (prefs === null || typeof prefs !== "object" || Array.isArray(prefs)) {
    throw diagnostic("webPreferences must be an object");
  }
  for (const [name, unsafe, safe] of [
    ["nodeIntegration", true, false],
    ["nodeIntegrationInWorker", true, false],
    ["nodeIntegrationInSubFrames", true, false],
    ["contextIsolation", false, true],
    ["webSecurity", false, true],
    ["allowRunningInsecureContent", true, false],
  ] as const) {
    const value = prefs[name];
    if (value === undefined) continue;
    if (typeof value !== "boolean" || value === unsafe) {
      throw diagnostic(`webPreferences.${name} must be ${safe}`);
    }
  }
  if (prefs.webviewTag !== undefined && prefs.webviewTag !== false) {
    throw diagnostic("webPreferences.webviewTag must be false");
  }
}

function ignoredOptionDiagnostics(options: BrowserWindowOptions): void {
  for (const name of ["backgroundColor", "icon"] as const) {
    if (options[name] !== undefined) {
      console.warn(
        `KELD window: ${name} is accepted but not yet applied by the native adapter`,
      );
    }
  }
  for (const name of ["disableBlinkFeatures", "spellcheck"] as const) {
    if (options.webPreferences?.[name] !== undefined) {
      console.warn(
        `KELD window: webPreferences.${name} is accepted but not yet applied`,
      );
    }
  }
  if (options.webPreferences?.preload !== undefined) {
    console.warn(
      "KELD window: preload execution is not yet implemented for this native window",
    );
  }
}

/** The first-proof content lifetime; loading and preload APIs are not faked. */
export class BrowserWindowWebContents extends EventEmitter {
  #destroyed = false;

  isDestroyed(): boolean {
    return this.#destroyed;
  }

  /** Native Destroyed is the only basis for publishing this tombstone. */
  dispose(): void {
    if (this.#destroyed) return;
    this.#destroyed = true;
    this.emit("destroyed");
  }
}

export class BrowserWindow extends EventEmitter {
  // Only getAllWindows may enter this synchronous, callback-free constructor
  // path. Reading a private ref from caller options would let a Proxy forge
  // the otherwise secret Symbol and bypass the Host Create contract.
  static #replaying: WindowRef | undefined;
  // Dispatch-order barrier only: Host/mirror still own pending-close state.
  // A recovered Created(pending_close) must emit 'close' before any later
  // window fact, even when public Ready and application listeners are deferred.
  static readonly #heldReplayCloses = new Map<
    number,
    { readonly ref: WindowRef; readonly closeSeq: bigint }
  >();
  readonly id!: number;
  readonly #ref!: WindowRef;
  readonly #contents = new BrowserWindowWebContents();
  #destroyed = false;
  #insideClosed = false;
  readonly #recovered!: boolean;
  #lastCloseSeq: bigint | undefined;

  constructor(options: BrowserWindowOptions = {}) {
    super();
    // Replay windows are materialized from trusted WindowSource records, not
    // from another Create request or a locally invented window identity.
    const replayed = BrowserWindow.#replaying;
    let adopted = false;
    let ref: WindowRef;
    if (replayed !== undefined) {
      ref = replayed;
    } else {
      checkOptions(options);
      const response = invokeWindowBlocking({
        variant: "Create",
        width: options.width ?? 800,
        height: options.height ?? 600,
        x: options.x ?? null,
        y: options.y ?? null,
      });
      if (response.variant !== "Created" && response.variant !== "Adopted") {
        throw diagnostic(
          `unexpected native Create response ${response.variant}`,
        );
      }
      ref = response.window;
      adopted = response.variant === "Adopted";
      const previous = windows.get(ref.id);
      if (
        response.variant === "Adopted" &&
        previous &&
        previous.#ref.incarnation === ref.incarnation &&
        !previous.isDestroyed()
      ) {
        return previous;
      }
    }
    const confirmed = getWindowSnapshot(ref);
    if (!confirmed || confirmed.host_destroyed) {
      throw diagnostic(
        "native Create did not deliver a matching host-owned window fact",
      );
    }
    if (windows.has(ref.id))
      throw diagnostic("Host reused an active window id unexpectedly");
    // TypeScript readonly is compile-time only. A writable public id can
    // redirect Map cleanup to a different Host window (including its listeners).
    Object.defineProperty(this, "id", {
      value: ref.id,
      enumerable: true,
      writable: false,
      configurable: false,
    });
    this.#ref = Object.freeze({ id: ref.id, incarnation: ref.incarnation });
    // An Adopted host window also predates this JS facade, even if application
    // code never called getAllWindows() to materialize its replay wrapper.
    this.#recovered = replayed !== undefined || adopted;
    windows.set(ref.id, this);
    if (replayed === undefined) {
      ignoredOptionDiagnostics(options);
      const event = { preventDefault(): void {} };
      emitNativeAppEvent("web-contents-created", event, this.#contents);
      emitNativeAppEvent("browser-window-created", event, this);
    }
  }

  static getAllWindows(): BrowserWindow[] {
    const snapshots = getWindowSnapshots().filter(
      (value) => !value.host_destroyed,
    );
    return snapshots.map(({ window }) => {
      const previous = windows.get(window.id);
      if (previous && previous.#ref.incarnation === window.incarnation)
        return previous;
      return BrowserWindow.#fromReplay(window);
    });
  }

  static #fromReplay(window: WindowRef): BrowserWindow {
    if (BrowserWindow.#replaying !== undefined) {
      throw diagnostic("recursive native window replay is not allowed");
    }
    BrowserWindow.#replaying = window;
    try {
      return new BrowserWindow();
    } finally {
      BrowserWindow.#replaying = undefined;
    }
  }

  static getFocusedWindow(): BrowserWindow | null {
    const snapshots = getWindowSnapshots();
    const focused = snapshots.find(
      (value) => !value.host_destroyed && value.state.focused,
    );
    return focused
      ? (BrowserWindow.getAllWindows().find(
          (win) => win.id === focused.window.id,
        ) ?? null)
      : null;
  }

  get webContents(): BrowserWindowWebContents {
    if (this.#destroyed && !this.#insideClosed) {
      throw new Error("KELD-COMPAT-011: Object has been destroyed");
    }
    return this.#contents;
  }

  #state() {
    if (this.#destroyed)
      throw new Error("KELD-COMPAT-011: Object has been destroyed");
    const value = getWindowSnapshot(this.#ref);
    if (!value || value.host_destroyed)
      throw new Error("KELD-COMPAT-011: Object has been destroyed");
    return value.state;
  }

  getSize(): number[] {
    const { width, height } = this.#state().bounds;
    return [width, height];
  }

  getPosition(): number[] {
    const { x, y } = this.#state().bounds;
    return [x, y];
  }

  isMaximized(): boolean {
    return this.#state().maximized;
  }
  isFullScreen(): boolean {
    return this.#state().full_screen;
  }
  isDestroyed(): boolean {
    return this.#destroyed;
  }

  #change(request: WindowRequest): void {
    if (this.#destroyed)
      throw new Error("KELD-COMPAT-011: Object has been destroyed");
    void invokeWindow(request).catch((error: unknown) => {
      console.error("KELD native window request failed:", error);
    });
  }

  maximize(): void {
    this.#change({
      variant: "SetMaximized",
      window: this.#ref,
      maximized: true,
    });
  }

  setFullScreen(flag: boolean): void {
    if (typeof flag !== "boolean")
      throw diagnostic("setFullScreen expects a boolean");
    this.#change({
      variant: "SetFullScreen",
      window: this.#ref,
      full_screen: flag,
    });
  }

  /** A request only: the native window stays until the app's verdict permits closing. */
  close(): void {
    if (this.#destroyed) return;
    this.#change({ variant: "RequestClose", window: this.#ref });
  }

  /** Host confirms native release before this synchronous method reports closed. */
  destroy(): void {
    if (this.#destroyed) return;
    const response = invokeWindowBlocking({
      variant: "Destroy",
      window: this.#ref,
    });
    if (
      response.variant !== "Destroyed" ||
      !getWindowSnapshot(this.#ref)?.host_destroyed
    ) {
      throw diagnostic(
        "native Destroy did not confirm the actual window release",
      );
    }
    this.#closed(true);
  }

  #closed(fromDestroy: boolean): void {
    if (this.#destroyed) return;
    this.#destroyed = true;
    // The private Host pair, never a user-visible property, owns cleanup.
    if (windows.get(this.#ref.id) === this) windows.delete(this.#ref.id);
    if (!fromDestroy) this.#contents.dispose();
    this.#insideClosed = true;
    try {
      this.emit("closed");
    } catch (error) {
      // A throwing application listener cannot interrupt physical release or
      // suppress the content tombstone; the host already destroyed the window.
      console.error("KELD window closed callback failed:", error);
    } finally {
      this.#insideClosed = false;
      if (fromDestroy) queueMicrotask(() => this.#contents.dispose());
    }
  }

  #nativeClose(close_seq: bigint): void {
    if (this.#destroyed || this.#lastCloseSeq === close_seq) return;
    this.#lastCloseSeq = close_seq;
    // A recovered wrapper may be materialized by a native fact before app code
    // has acquired it or registered its unsaved-document listener. Veto rather
    // than allowing an invisible/default close through that recovery interval.
    let prevented = this.#recovered && this.listenerCount("close") === 0;
    const event = {
      get defaultPrevented(): boolean {
        return prevented;
      },
      preventDefault(): void {
        prevented = true;
      },
    };
    try {
      this.emit("close", event);
    } catch (error) {
      // A throwing application callback must not cause data loss or wedge ClosePending.
      prevented = true;
      console.error("KELD window close callback failed; vetoed:", error);
    }
    void invokeWindow({
      variant: "CloseReply",
      window: this.#ref,
      close_seq,
      verdict: prevented ? "Veto" : "Allow",
    }).catch((error: unknown) => {
      console.error("KELD native close verdict failed:", error);
    });
  }

  /** Host facts are applied before this method; never mint a local identity. */
  static #dispatchNativeClose(ref: WindowRef, close_seq: bigint): void {
    let confirmed;
    try {
      confirmed = getWindowSnapshot(ref);
    } catch (error) {
      // The role/link has ended, so a late scheduled delivery has no legal
      // successor owner and must never send an Allow on its dead generation.
      console.error(
        "KELD close verdict abandoned after Host mirror failure:",
        error,
      );
      return;
    }
    if (
      !confirmed ||
      confirmed.host_destroyed ||
      confirmed.pending_close !== close_seq
    ) {
      return;
    }
    const previous = windows.get(ref.id);
    // A different incarnation is an ownership mismatch, not permission to
    // replace the caller's JS listeners with a new, unrelated wrapper.
    if (previous && previous.#ref.incarnation !== ref.incarnation) return;
    const window = previous ?? BrowserWindow.#fromReplay(ref);
    window.#nativeClose(close_seq);
  }

  static #flushHeldReplayClose(ref: WindowRef): void {
    if (!hostApp.isReady()) return;
    const held = BrowserWindow.#heldReplayCloses.get(ref.id);
    if (!held || held.ref.incarnation !== ref.incarnation) return;
    BrowserWindow.#heldReplayCloses.delete(ref.id);
    BrowserWindow.#dispatchNativeClose(held.ref, held.closeSeq);
  }

  /** Replay can arrive before public Ready; give the successor its Ready turn. */
  static #receiveNativeClose(
    ref: WindowRef,
    close_seq: bigint,
    fromReplay: boolean,
  ): void {
    if (fromReplay) {
      BrowserWindow.#heldReplayCloses.set(ref.id, {
        ref: { id: ref.id, incarnation: ref.incarnation },
        closeSeq: close_seq,
      });
    }
    if (!fromReplay && hostApp.isReady()) {
      BrowserWindow.#flushHeldReplayClose(ref);
      BrowserWindow.#dispatchNativeClose(ref, close_seq);
      return;
    }
    void hostApp.whenReady().then(
      () => {
        // Ready continuations can attach a close/veto listener to the adopted
        // window before the gap's held close is delivered. A later native fact
        // flushes it synchronously first, preserving the Host's fact order.
        setImmediate(() => {
          try {
            if (fromReplay) BrowserWindow.#flushHeldReplayClose(ref);
            else {
              BrowserWindow.#flushHeldReplayClose(ref);
              BrowserWindow.#dispatchNativeClose(ref, close_seq);
            }
          } catch (error) {
            // Expected shutdowns are handled by #dispatchNativeClose; do not
            // let an independent late callback crash the event loop.
            console.error("KELD deferred native close dispatch failed:", error);
          }
        });
      },
      (error: unknown) => {
        // A dead link ends the Host's generation, so no stale Allow is sent.
        if (fromReplay) BrowserWindow.#heldReplayCloses.delete(ref.id);
        console.error(
          "KELD recovery close lost its Host Ready barrier:",
          error,
        );
      },
    );
  }

  static {
    onWindowFactApplied((fact: WindowEvent) => {
      if (fact.variant === "Created" && fact.pending_close !== null) {
        BrowserWindow.#receiveNativeClose(
          fact.window,
          fact.pending_close,
          true,
        );
        return;
      }
      // An already-applied Bounds/Focus/FullScreen/Destroyed fact may dispatch
      // before the deferred Ready continuation for an earlier replay Created.
      // Emit that replay close first, never reverse the host's causal order.
      BrowserWindow.#flushHeldReplayClose(fact.window);
      if (fact.variant === "CloseRequested") {
        BrowserWindow.#receiveNativeClose(fact.window, fact.close_seq, false);
        return;
      }
      const window = windows.get(fact.window.id);
      if (!window || window.#ref.incarnation !== fact.window.incarnation)
        return;
      switch (fact.variant) {
        case "Bounds":
          window.emit("resize");
          break;
        case "Maximized":
          window.emit(fact.maximized ? "maximize" : "unmaximize");
          break;
        case "Destroyed":
          window.#closed(false);
          break;
        default:
          break;
      }
    });
  }
}
