---
name: logging
description: Wide event logging pattern for structured, queryable logs.
agents: []
pre-loaded: [backend_engineer, frontend_engineer, automation_engineer]
---

# Logging

## What is a Wide Event?

A wide event is a single structured log entry that captures the complete context of one request through one service. Traditional logging scatters many small log lines through a handler — "request received," "user fetched," "payment processed" — which are disconnected and impossible to query together. A wide event builds one rich object as the request progresses and emits it exactly once at completion, whether the request succeeds or fails.

The result: every request becomes fully queryable by any combination of its fields. Instead of "checkout broke," you can answer "which premium users with cart values over $100 had payment failures after the v2.4.1 deploy?" — without adding any new logging code.

## The Pattern

```typescript
app.post('/checkout', async (c) => {
  const startTime = Date.now();
  const wideEvent: Record<string, unknown> = {
    method: c.req.method,
    path: c.req.path,
    request_id: c.get('requestId') ?? crypto.randomUUID(),
    timestamp: new Date().toISOString(),
  };

  try {
    const user = await getUser(c.get('userId'));
    wideEvent.user = { id: user.id, subscription: user.subscription };

    const cart = await getCart(user.id);
    wideEvent.cart = { total_cents: cart.total, item_count: cart.items.length };

    const order = await createOrder(cart);
    wideEvent.order = { id: order.id };

    wideEvent.status_code = 201;
    wideEvent.outcome = 'success';
    return c.json(order, 201);
  } catch (error) {
    wideEvent.status_code = 500;
    wideEvent.outcome = 'error';
    wideEvent.error = { type: error.name, message: error.message };
    throw error;
  } finally {
    wideEvent.duration_ms = Date.now() - startTime;
    logger.info(wideEvent);
  }
});
```

Use middleware to handle infrastructure fields (timing, status, environment, emission) so handlers only add business context.

## What to Include

**Always present:**
- `request_id`, `method`, `path`, `timestamp`, `duration_ms`, `status_code`, `outcome`
- Environment: `service`, `version`, `commit_hash`, `region` — set once at startup via `pino` base config, injected automatically

**Business context** (add as your handler progresses):
- User fields: `id`, `subscription`, `account_age_days` — high cardinality, enables per-user debugging
- Domain-specific: cart totals, order IDs, feature flags — include what changes the priority of a failure

**On error**, always add: `error.type`, `error.message`

## Structure

**Single logger** — configure once in `lib/logger.ts` using `pino`, import everywhere. Never `new Logger()` per file, never `console.log`.

```typescript
// lib/logger.ts
export const logger = pino({
  level: process.env.LOG_LEVEL ?? 'info',
  base: {
    service: process.env.SERVICE_NAME,
    version: process.env.SERVICE_VERSION,
    commit_hash: process.env.COMMIT_SHA,
    region: process.env.AWS_REGION,
    environment: process.env.NODE_ENV,
  },
});
```

**Two log levels only:** `info` for all wide events, `error` for unexpected failures. If you want a debug log, add the data to the wide event instead.

**JSON only.** Never emit unstructured strings — `logger.info('user logged in')` is unqueryable. Put the data in the event: `wideEvent.auth = { user_id, method: 'oauth' }`.

## Anti-Patterns

- Multiple `console.log` calls per request — consolidate into the wide event
- Logging only technical details — always include business context (who, how much, what plan)
- Creating logger instances per file — import the single shared logger
- Unstructured string messages — every field must be queryable
