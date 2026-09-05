/** Preserve edit order for each preset even when HTTP responses are slow.
 * Different presets can save independently; a rejected edit does not poison
 * the next request. The caller receives every error for visible feedback. */
export class PresetWriteQueue {
  private pending = new Map<string, Promise<unknown>>();
  constructor(private persist: (name: string, patch: Record<string, unknown>) => Promise<unknown>) {}
  idle(name: string): Promise<unknown> { return this.pending.get(name)?.catch(() => undefined) ?? Promise.resolve(); }
  write(name: string, patch: Record<string, unknown>): Promise<unknown> {
    const previous = this.pending.get(name);
    let next: Promise<unknown>;
    try {
      next = previous ? previous.catch(() => undefined).then(() => this.persist(name, patch)) : Promise.resolve(this.persist(name, patch));
    } catch (error) {
      next = Promise.reject(error);
    }
    this.pending.set(name, next);
    const cleanup = () => { if (this.pending.get(name) === next) this.pending.delete(name); };
    next.then(cleanup, cleanup);
    return next;
  }
}
