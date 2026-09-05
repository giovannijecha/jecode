// Host capabilities shared by foreground command and turn workflows.

import type { Session } from "../session.ts";
import type { SessionPermissions } from "../permissions.ts";
import type { Activity } from "./activity.ts";
import type { AppState } from "./app-state.ts";
import type { Block, NoticeBlock } from "./blocks.ts";
import type { FeedbackController } from "./feedback.ts";

export type WorkflowOptions = {
  session: Session;
  transcriptRoot: string;
  state: AppState;
  permissions: SessionPermissions;
  feedback: FeedbackController;
  emit(block: Block): void;
  commandNotice(notice: NoticeBlock): void;
  render(block?: Block): void;
  replaceTranscript(): void;
  refreshSettings(): void;
  startActivity(kind: Activity["kind"], label: string): Activity | undefined;
  finishActivity(activity: Activity): void;
};
