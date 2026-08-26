import type { TyselApp } from "@tysel/types";

interface Job {
  id: string;
  action: string;
}

export default {
  async fetch() {
    return Response.json({ queues: ["jobs"], cron: ["heartbeat"] });
  },
  tasks: {
    heartbeat: {
      kind: "cron",
      expression: "*/5 * * * *",
      async handler(context) {
        console.log(JSON.stringify({
          event: "heartbeat",
          requestId: context.requestId,
          deadlineMs: context.deadlineMs,
        }));
      },
    },
    processJob: {
      kind: "queue",
      name: "jobs",
      async handler(
        message: Job,
        context,
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
} satisfies TyselApp;
