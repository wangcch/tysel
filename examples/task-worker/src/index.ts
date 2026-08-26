import type { RequestContext } from "@tysel/types";

interface Job {
  id: string;
  action: string;
}

export default {
  async fetch(): Promise<Response> {
    return Response.json({ queues: ["jobs"], cron: ["heartbeat"] });
  },
  tasks: {
    heartbeat: {
      kind: "cron" as const,
      expression: "*/5 * * * *",
      async handler(context: RequestContext) {
        console.log(JSON.stringify({
          event: "heartbeat",
          requestId: context.requestId,
          deadlineMs: context.deadlineMs,
        }));
      },
    },
    processJob: {
      kind: "queue" as const,
      name: "jobs",
      async handler(
        message: Job,
        context: RequestContext,
      ) {
        return {
          accepted: true,
          jobId: message.id,
          action: message.action,
          requestId: context.requestId,
        };
      },
    },
  },
};
