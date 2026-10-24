/**
 * Electron-shaped first-window surface over the one authenticated Keld app link.
 * The Core registry is the sole owner of ids, native state and close decisions.
 * No renderer/Node grant, alternate transport, or optimistic native state exists.
 */
import { EventEmitter } from "node:events";
import {
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
  readonly id!: number;
  readonly #ref!: WindowRef;
  readonly #contents = new BrowserWindowWebContents();
  #destroyed = false;
  #insideClosed = false;

  constructor(options: BrowserWindowOptions = {}) {
    super();
    // Replay windows are materialized from trusted WindowSource records, not
    // from another Create request or a locally invented window identity.
    const replayed = BrowserWindow.#replaying;
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
    this.id = ref.id;
    this.#ref = ref;
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
    windows.delete(this.id);
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
    if (this.#destroyed) return;
    let prevented = false;
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

  static {
    onWindowFactApplied((fact: WindowEvent) => {
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
        case "CloseRequested":
          window.#nativeClose(fact.close_seq);
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
