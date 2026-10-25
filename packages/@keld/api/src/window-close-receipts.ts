/**
 * Only authenticated *dispatched* close facts qualify for late-import catch-up.
 * The WorkerLink may apply later Host facts ahead of event dispatch while a
 * synchronous caller is parked. A mirror snapshot alone has no proof that the
 * CloseRequested callback reached this role before earlier Bounds callbacks.
 */
import type { WindowEvent, WindowRef } from "./window-codec.ts";

export interface DeliveredWindowClose {
  readonly window: Readonly<WindowRef>;
  readonly closeSeq: bigint;
}

export class WindowCloseReceipts {
  readonly #records = new Map<
    number,
    { readonly ref: Readonly<WindowRef>; pending: bigint | null }
  >();

  observe(fact: WindowEvent): void {
    const { id, incarnation } = fact.window;
    if (fact.variant === "Created") {
      this.#records.set(id, {
        ref: Object.freeze({ id, incarnation }),
        pending: fact.pending_close,
      });
      return;
    }
    const earlier = this.#records.get(id);
    if (!earlier || earlier.ref.incarnation !== incarnation) return;
    if (fact.variant === "Destroyed") this.#records.delete(id);
    if (fact.variant === "CloseRequested") earlier.pending = fact.close_seq;
  }

  pending(): readonly DeliveredWindowClose[] {
    const result: DeliveredWindowClose[] = [];
    for (const record of this.#records.values()) {
      if (record.pending !== null) {
        result.push(
          Object.freeze({ window: record.ref, closeSeq: record.pending }),
        );
      }
    }
    return Object.freeze(result);
  }
}
