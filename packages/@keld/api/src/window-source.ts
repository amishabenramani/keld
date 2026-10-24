/** Internal host-fact mirror (GH-531 §4.d); deliberately not attached to a link. */
import { WINDOW_CHANNEL, linkError } from "../../kipc/src/transport.ts";
import type { MirrorSource } from "./mirror.ts";
import {
  decodeWindowEvent,
  type Rect, type WindowEvent, type WindowRef, type WindowState,
} from "./window-codec.ts";

/** Source-owned immutable host facts; Electron-facing tombstones are separate. */
export interface WindowSnapshot {
  readonly window: Readonly<WindowRef>;
  readonly state: Readonly<Omit<WindowState, "bounds">> & { readonly bounds: Readonly<Rect> };
  readonly pending_close: bigint | null;
  readonly host_destroyed: boolean;
}

function snapshot(window: WindowRef, state: WindowState, pending_close: bigint | null, host_destroyed: boolean): WindowSnapshot {
  // Copy every object: neither retained dispatch facts nor caller snapshots own
  // aliases into mutable map state, or into the transport's payload storage.
  return Object.freeze({
    window: Object.freeze({ id: window.id, incarnation: window.incarnation }),
    state: Object.freeze({
      bounds: Object.freeze({ x: state.bounds.x, y: state.bounds.y, width: state.bounds.width, height: state.bounds.height }),
      focused: state.focused, maximized: state.maximized, full_screen: state.full_screen,
    }),
    pending_close,
    host_destroyed,
  });
}

/**
 * No socket, identity minting, permission policy, optimistic write or event queue.
 * A largest decoded Created fact has four plain objects, fourteen own properties:
 * eleven scalar slots and three object references. Its separate mirror snapshot
 * has the same object/property/slot counts. All integers are wire-bounded; variant
 * strings are closed literals. No payload, buffer, view or variable string survives.
 * These shapes are not a heap-byte budget: production attachment still requires
 * independent retained-fact, map-census, heap and alias/lifetime qualification.
 */
export class WindowSource implements MirrorSource<WindowEvent> {
  readonly channel = WINDOW_CHANNEL;
  readonly #windows = new Map<number, WindowSnapshot>();
  readonly #onFact: (fact: WindowEvent) => void;
  #failure: Error | undefined;

  constructor(onFact: (fact: WindowEvent) => void) { this.#onFact = onFact; }

  #requireLive(): void { if (this.#failure) throw this.#failure; }

  #reject(): Error {
    this.#failure ??= linkError("KELD-IPC-005", "window mirror rejected a host fact. Check the host window pair and codec before restarting the role.");
    return this.#failure;
  }

  /** Decode once through the shared codec, freezing its bounded known graph. */
  decode(payload: Uint8Array): WindowEvent {
    this.#requireLive();
    try {
      const fact = decodeWindowEvent(payload);
      Object.freeze(fact.window);
      if (fact.variant === "Created") {
        Object.freeze(fact.state.bounds);
        Object.freeze(fact.state);
      } else if (fact.variant === "Bounds") {
        Object.freeze(fact.bounds);
      }
      return Object.freeze(fact);
    } catch {
      throw this.#reject();
    }
  }

  /** Apply only host-confirmed facts; Created first, exact pair, Destroyed last. */
  apply(fact: WindowEvent): void {
    this.#requireLive();
    try {
      if (fact.window.id === 0 || fact.window.incarnation === 0n) throw this.#reject();
      const previous = this.#windows.get(fact.window.id);
      if (fact.variant === "Created") {
        if (previous) throw this.#reject();
        this.#windows.set(fact.window.id, snapshot(fact.window, fact.state, fact.pending_close, false));
        return;
      }
      if (!previous || previous.window.incarnation !== fact.window.incarnation || previous.host_destroyed) throw this.#reject();
      let state: WindowState = previous.state;
      let pending_close = previous.pending_close;
      let host_destroyed = false;
      switch (fact.variant) {
        case "Bounds": state = { ...state, bounds: fact.bounds }; break;
        case "Focus": state = { ...state, focused: fact.focused }; break;
        case "Maximized": state = { ...state, maximized: fact.maximized }; break;
        case "FullScreen": state = { ...state, full_screen: fact.full_screen }; break;
        case "CloseRequested": pending_close = fact.close_seq; break;
        case "Destroyed": pending_close = null; host_destroyed = true; break;
        default: throw this.#reject();
      }
      this.#windows.set(fact.window.id, snapshot(fact.window, state, pending_close, host_destroyed));
    } catch {
      throw this.#reject();
    }
  }

  /** Existing dispatch owns ordering and isolates listener exceptions. */
  dispatch(fact: WindowEvent): void {
    // Earlier applied records still drain after a later source fact fails.
    this.#onFact(fact);
  }

  /** A local read only; a wrong pair never observes a different incarnation. */
  get(window: WindowRef): WindowSnapshot | undefined {
    this.#requireLive();
    const value = this.#windows.get(window.id);
    return value?.window.incarnation === window.incarnation ? value : undefined;
  }

  /** Immutable host snapshots, including host-destroyed entries, in host-id order. */
  all(): readonly WindowSnapshot[] {
    this.#requireLive();
    return Object.freeze([...this.#windows.values()].sort((a, b) => a.window.id - b.window.id));
  }

  /** Called by the future link owner only after its retained-record drain. */
  end(error: Error): void {
    this.#failure ??= error;
    this.#windows.clear();
  }
}
