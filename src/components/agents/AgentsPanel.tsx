import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  AlertTriangle,
  ChevronsUpDown,
  FileText,
  Info,
  KeyRound,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  Trash2,
  Undo2,
  Users,
  Workflow,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import {
  codexAgentWorkflowApi,
  providersApi,
  type CodexAgentWorkflowRoleAgents,
  type CodexAgentWorkflowStatus,
  type CodexSubagent,
  type CodexSubagentModelCandidate,
  type CodexSubagentSandboxMode,
} from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import ApiKeyInput from "@/components/providers/forms/ApiKeyInput";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { cn } from "@/lib/utils";
import { extractCodexModelName } from "@/utils/providerConfigUtils";
import {
  getCodexMemberCredentials,
  getCodexMemberWireApi,
  type CodexMemberWireApi,
} from "@/utils/aggregateProvider";
import { extractErrorMessage } from "@/utils/errorUtils";
import type { Provider } from "@/types";

// 固定官方 6 档（无 minimal），新建默认 high。
const REASONING_EFFORTS = ["low", "medium", "high", "xhigh", "max", "ultra"];

// 官方内置 subagent 角色：worker（实现/执行）、explorer（只读探索）、default（通用）。
const AGENT_TYPES = ["worker", "explorer", "default"] as const;
type AgentType = (typeof AGENT_TYPES)[number];

interface AgentsPanelProps {
  onOpenChange?: (open: boolean) => void;
}

export function AgentsPanel(_props: AgentsPanelProps) {
  const { t } = useTranslation();
  const [subagents, setSubagents] = useState<CodexSubagent[]>([]);
  const [subagentsLoading, setSubagentsLoading] = useState(true);
  const [subagentsLoadFailed, setSubagentsLoadFailed] = useState(false);

  const [status, setStatus] = useState<CodexAgentWorkflowStatus | null>(null);
  const [statusLoading, setStatusLoading] = useState(true);

  const [roleAgents, setRoleAgents] = useState<CodexAgentWorkflowRoleAgents>({
    worker: [],
    explorer: [],
    default: [],
  });
  const roleTouched = useRef(false);
  const [installing, setInstalling] = useState(false);
  const [cancelling, setCancelling] = useState(false);

  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<CodexSubagent | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<CodexSubagent | null>(
    null,
  );
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);

  const loadSubagents = useCallback(async () => {
    setSubagentsLoading(true);
    setSubagentsLoadFailed(false);
    try {
      setSubagents(await codexAgentWorkflowApi.listSubagents());
    } catch (error) {
      console.error("[AgentsPanel] Failed to list registered subagents", error);
      setSubagentsLoadFailed(true);
      toast.error(
        t("agents.subagentsLoadFailed", {
          defaultValue: "Failed to load registered subagents",
        }),
      );
    } finally {
      setSubagentsLoading(false);
    }
  }, [t]);

  const loadStatus = useCallback(async () => {
    setStatusLoading(true);
    try {
      setStatus(await codexAgentWorkflowApi.getWorkflowStatus());
    } catch (error) {
      console.error("[AgentsPanel] Failed to load workflow status", error);
      toast.error(
        t("agents.loadFailed", { defaultValue: "Failed to load agent status" }),
      );
    } finally {
      setStatusLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void loadSubagents();
    void loadStatus();
  }, [loadSubagents, loadStatus]);

  const managedSubagents = useMemo(
    () =>
      subagents.filter((agent) => agent.managed && agent.available !== false),
    [subagents],
  );

  // 已注册列表或已安装状态变化时保持角色映射有效：过滤掉已不再注册的 agent；
  // 用户主动修改后（roleTouched），refresh / install 不再覆盖。
  useEffect(() => {
    if (roleTouched.current) return;
    if (managedSubagents.length === 0) {
      setRoleAgents({ worker: [], explorer: [], default: [] });
      return;
    }
    const valid = (names: string[] | undefined) =>
      (names ?? []).filter((name) =>
        managedSubagents.some((agent) => agent.name === name),
      );
    const hasStatusRoles = Boolean(
      status?.roleAgents &&
        (status.roleAgents.worker.length ||
          status.roleAgents.explorer.length ||
          status.roleAgents.default.length),
    );
    let next: CodexAgentWorkflowRoleAgents;
    if (hasStatusRoles && status) {
      next = {
        worker: valid(status.roleAgents.worker),
        explorer: valid(status.roleAgents.explorer),
        default: valid(status.roleAgents.default),
      };
    } else if (status?.workerAgents?.length) {
      // 旧 manifest：worker_agents 整体归入 worker 角色。
      next = { worker: valid(status.workerAgents), explorer: [], default: [] };
    } else if (status?.workerAgent) {
      next = { worker: valid([status.workerAgent]), explorer: [], default: [] };
    } else {
      next = { worker: [], explorer: [], default: [] };
    }
    if (roleUnion(next).length === 0) {
      next = { worker: [managedSubagents[0].name], explorer: [], default: [] };
    }
    setRoleAgents(next);
  }, [managedSubagents, status]);

  const selectedWorker = useMemo(
    () =>
      subagents.find((agent) => agent.name === deriveWorkerName(roleAgents)) ??
      null,
    [subagents, roleAgents],
  );

  const workflowUsesSubagent = useCallback(
    (name: string) => roleUnion(roleAgents).includes(name),
    [roleAgents],
  );

  const toggleRole = async (
    role: keyof CodexAgentWorkflowRoleAgents,
    name: string,
    checked: boolean,
  ) => {
    roleTouched.current = true;
    const next: CodexAgentWorkflowRoleAgents = {
      ...roleAgents,
      [role]: checked
        ? [...roleAgents[role], name]
        : roleAgents[role].filter((n) => n !== name),
    };
    setRoleAgents(next);
    // 已安装时即时按新映射重新生成 Skill；所有角色都清空时不重装。
    if (status?.installed && roleUnion(next).length > 0) {
      try {
        setStatus(
          await codexAgentWorkflowApi.installWorkflow({
            workerAgent: deriveWorkerName(next),
            workerAgents: roleUnion(next),
            roleAgents: next,
          }),
        );
      } catch (error) {
        console.error("[AgentsPanel] Failed to reinstall workflow", error);
      }
    }
  };

  const installWorkflow = async () => {
    if (!selectedWorker) return;
    setInstalling(true);
    try {
      const next = await codexAgentWorkflowApi.installWorkflow({
        workerAgent: selectedWorker.name,
        workerAgents: roleUnion(roleAgents),
        roleAgents,
      });
      setStatus(next);
      toast.success(
        t("agents.workflowInjectedSuccess", {
          defaultValue: "Workflow skill installed",
        }),
      );
    } catch (error) {
      console.error(
        "[AgentsPanel] Failed to install Codex agent workflow",
        error,
      );
      toast.error(
        t("agents.workflowInstallFailed", {
          defaultValue: "Failed to install workflow",
        }),
      );
    } finally {
      setInstalling(false);
    }
  };

  const cancelInstructions = async () => {
    setCancelling(true);
    try {
      const next = await codexAgentWorkflowApi.cancelWorkflowInstructions();
      setStatus(next);
      toast.success(
        t("agents.cancelInstructionsSuccess", {
          defaultValue: "Workflow skill uninstalled — files restored",
        }),
      );
    } catch (error) {
      console.error("[AgentsPanel] Failed to uninstall workflow skill", error);
      toast.error(
        t("agents.cancelInstructionsFailed", {
          defaultValue: "Failed to uninstall workflow skill",
        }),
      );
    } finally {
      setCancelling(false);
    }
  };

  const deleteSubagent = async () => {
    if (!confirmDelete) return;
    const target = confirmDelete;
    if (workflowUsesSubagent(target.name)) {
      toast.warning(
        t("agents.subagentDeleteBlockedByWorkflow", {
          defaultValue:
            "This subagent is selected in the Workflow Skill. Deselect it before deleting.",
        }),
      );
      setConfirmDelete(null);
      return;
    }
    setDeleting(true);
    try {
      try {
        await codexAgentWorkflowApi.deleteSubagent(target.name);
      } catch (deleteError) {
        const deleteDetail = extractErrorMessage(deleteError);
        console.error(
          "[AgentsPanel] Failed to delete subagent",
          deleteDetail || deleteError,
        );
        toast.error(
          deleteDetail
            ? t("agents.subagentDeleteFailedDetail", {
                defaultValue: "Failed to delete subagent: {{detail}}",
                detail: deleteDetail,
              })
            : t("agents.subagentDeleteFailed", {
                defaultValue: "Failed to delete subagent",
              }),
        );
        return;
      }
      toast.success(
        t("agents.subagentDeleted", { defaultValue: "Subagent deleted" }),
      );
      setConfirmDelete(null);
      await loadSubagents();
    } finally {
      setDeleting(false);
    }
  };

  const busy = installing || cancelling || deleting;

  const statusLine = statusLoading
    ? {
        dot: "bg-muted-foreground",
        text: t("agents.loading", { defaultValue: "Loading status..." }),
      }
    : !status
      ? null
      : !status.installed
        ? {
            dot: "bg-amber-500",
            text: t("agents.notInstalledStatus", {
              defaultValue:
                "Not installed — assign worker/explorer/default roles and install the Workflow Skill below.",
            }),
          }
        : {
            dot: "bg-emerald-500",
            text: t("agents.installedStatus", {
              defaultValue:
                "Installed — the Workflow Skill is installed into Codex skills. Invoke it explicitly with @Cube Dispatch in any Codex conversation.",
            }),
          };

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-6 pb-6">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-3">
        <Card className="rounded-xl border-border shadow-sm">
          <CardHeader className="px-4 py-3">
            <div className="flex items-start justify-between gap-3">
              <div className="flex min-w-0 items-start gap-3">
                <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                  <Workflow className="h-4 w-4" />
                </span>
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <h1 className="text-base font-semibold">
                      {t("agents.title", {
                        defaultValue: "Collaborative Workflow",
                      })}
                    </h1>
                    {status && (
                      <Badge variant="secondary">
                        {status.installed
                          ? t("agents.installedBadge", {
                              defaultValue: "Installed",
                            })
                          : t("agents.notInstalledBadge", {
                              defaultValue: "Not installed",
                            })}
                      </Badge>
                    )}
                  </div>
                  <p className="mt-0.5 whitespace-pre-line text-xs text-muted-foreground">
                    {t("agents.workflowDescription", {
                      defaultValue:
                        "Register cheap models as officially supported Codex subagents, pick one as the default worker, and install the Workflow Skill so Codex knows what each subagent is specialized in.",
                    })}
                  </p>
                  {statusLine && (
                    <p className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
                      <span
                        className={cn(
                          "h-1.5 w-1.5 shrink-0 rounded-full",
                          statusLine.dot,
                        )}
                      />
                      <span className="min-w-0 truncate">
                        {statusLine.text}
                      </span>
                    </p>
                  )}
                </div>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  void loadStatus();
                  void loadSubagents();
                }}
                disabled={statusLoading || subagentsLoading}
                aria-label={
                  statusLoading
                    ? t("agents.refreshing", {
                        defaultValue: "Refreshing status...",
                      })
                    : t("agents.refresh", { defaultValue: "Refresh status" })
                }
              >
                <RefreshCw
                  className={cn(
                    "h-4 w-4",
                    (statusLoading || subagentsLoading) && "animate-spin",
                  )}
                />
              </Button>
            </div>
          </CardHeader>
        </Card>

        <Tabs defaultValue="subagents">
          <TabsList className="grid w-full grid-cols-2 rounded-lg bg-muted p-1">
            <TabsTrigger value="subagents" className="min-w-0 gap-2">
              <Users className="h-4 w-4" />
              {t("agents.tabSubagents", {
                defaultValue: "Subagent Registration",
              })}
            </TabsTrigger>
            <TabsTrigger value="workflow" className="min-w-0 gap-2">
              <FileText className="h-4 w-4" />
              {t("agents.tabWorkflow", { defaultValue: "Workflow Skill" })}
            </TabsTrigger>
          </TabsList>
          <TabsContent value="subagents" className="mt-3">
            <Card className="rounded-xl border-border shadow-sm">
              <CardHeader className="px-4 py-3">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                      <Users className="h-3.5 w-3.5" />
                    </span>
                    <div className="min-w-0">
                      <h2 className="text-sm font-semibold">
                        {t("agents.registeredAgents", {
                          defaultValue: "Registered subagents",
                        })}
                      </h2>
                      <p className="text-xs text-muted-foreground">
                        {t("agents.registeredAgentsDesc", {
                          defaultValue:
                            "Register subagents with a worker/explorer/default role; the workflow dispatches to the matching role and records every subagent in the Workflow Skill.",
                        })}
                      </p>
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {!subagentsLoading && !subagentsLoadFailed && (
                      <Badge variant="outline">
                        {t("agents.subagentCount", {
                          defaultValue: "{{count}} subagents",
                          count: subagents.length,
                        })}
                      </Badge>
                    )}
                    <Button
                      size="sm"
                      onClick={() => {
                        setEditing(null);
                        setFormOpen(true);
                      }}
                    >
                      <Plus className="h-4 w-4" />
                      {t("agents.addSubagent", {
                        defaultValue: "Add subagent",
                      })}
                    </Button>
                  </div>
                </div>
              </CardHeader>
              <CardContent className="flex flex-col gap-2 px-4 pb-4 pt-0">
                <div className="flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 dark:border-amber-700/60 dark:bg-amber-900/20">
                  <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />
                  <div className="min-w-0 text-xs leading-snug text-amber-700 dark:text-amber-300">
                    <p className="font-semibold">
                      {t("agents.subagentRoutingRequiredTitle", {
                        defaultValue:
                          "Local routing required for custom subagents",
                      })}
                    </p>
                    <p className="mt-0.5">
                      {t("agents.subagentRoutingRequiredBody", {
                        defaultValue:
                          "Multi-agent v2 encrypts dispatched tasks. Without local routing enabled, Codex cannot deliver plaintext tasks to your custom subagents — delegation fails with empty or unreadable messages. Enable local routing (Proxy page → Local proxy takeover) before registering or using custom subagents.",
                      })}
                    </p>
                  </div>
                </div>
                <div className="flex items-start gap-2 rounded-lg border border-border bg-muted/40 px-3 py-2">
                  <Info className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
                  <div className="min-w-0 text-xs leading-snug text-muted-foreground">
                    <p className="font-semibold">
                      {t("agents.subagentResponsesOnlyTitle", {
                        defaultValue: "Responses protocol only",
                      })}
                    </p>
                    <p className="mt-0.5">
                      {t("agents.subagentResponsesOnlyBody", {
                        defaultValue:
                          "Custom subagents only support the Responses protocol. Chat Completions / Anthropic are not supported because Codex collaboration subagent task delivery relies on Responses plaintext handling.",
                      })}
                    </p>
                  </div>
                </div>
                {subagentsLoading ? (
                  <div className="flex items-center gap-2 rounded-lg bg-muted/30 px-4 py-4 text-sm text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    {t("agents.loading", { defaultValue: "Loading status..." })}
                  </div>
                ) : subagentsLoadFailed ? (
                  <div className="flex flex-col items-start gap-2 rounded-lg bg-muted/30 px-4 py-4">
                    <p className="text-sm text-muted-foreground">
                      {t("agents.subagentsLoadFailed", {
                        defaultValue: "Failed to load registered subagents",
                      })}
                    </p>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => void loadSubagents()}
                    >
                      {t("agents.retry", { defaultValue: "Retry" })}
                    </Button>
                  </div>
                ) : subagents.length === 0 ? (
                  <div className="rounded-lg bg-muted/30 px-4 py-4">
                    <p className="text-sm font-medium">
                      {t("agents.subagentsEmpty", {
                        defaultValue: "No subagents registered yet",
                      })}
                    </p>
                    <p className="mt-0.5 text-xs text-muted-foreground">
                      {t("agents.subagentsEmptyHint", {
                        defaultValue:
                          "Register one or more subagents with a worker/explorer/default role, then assign the roles in the Workflow Skill below.",
                      })}
                    </p>
                  </div>
                ) : (
                  <ul className="flex flex-col gap-1.5">
                    {subagents.map((agent) => (
                      <li
                        key={agent.name}
                        className="flex items-start gap-2 rounded-lg border border-border bg-background px-3 py-2"
                      >
                        <div className="min-w-0 flex-1">
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="min-w-0 break-words text-sm font-semibold">
                              {agent.name}
                            </span>
                            <Badge
                              variant="secondary"
                              className="max-w-48 truncate"
                            >
                              {agent.model}
                            </Badge>
                            <Badge variant="outline">
                              {agentTypeLabel(agent.agentType, t)}
                            </Badge>
                            {!agent.managed && (
                              <Badge variant="outline">
                                {t("agents.unmanagedBadge", {
                                  defaultValue: "Needs adoption",
                                })}
                              </Badge>
                            )}
                            {agent.managed && agent.available === false && (
                              <Badge variant="destructive">
                                {t("agents.unavailableBadge", {
                                  defaultValue: "Provider missing",
                                })}
                              </Badge>
                            )}
                            <Badge variant="outline">
                              {sandboxLabel(agent.sandboxMode, t)}
                            </Badge>
                            <Badge variant="outline">
                              {agent.reasoningEffort}
                            </Badge>
                          </div>
                          {agent.description && (
                            <p
                              className="mt-0.5 truncate text-xs text-muted-foreground"
                              title={agent.description}
                            >
                              {agent.description}
                            </p>
                          )}
                          <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                            <FileText className="h-3 w-3 shrink-0" />
                            <span
                              className="min-w-0 truncate font-mono"
                              title={agent.agentPath}
                            >
                              {agent.agentPath}
                            </span>
                          </div>
                          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                            <span className="max-w-40 truncate">
                              {agent.modelProviderId}
                            </span>
                            <span
                              className="max-w-56 truncate"
                              title={agent.modelBaseUrl}
                            >
                              {agent.modelBaseUrl}
                            </span>
                            <span className="flex items-center gap-1">
                              <KeyRound className="h-3 w-3" />
                              {agent.apiKey != null
                                ? t("agents.keyConfigured", {
                                    defaultValue: "Configured",
                                  })
                                : t("common.notSet", {
                                    defaultValue: "Not set",
                                  })}
                            </span>
                          </div>
                        </div>
                        <div className="flex shrink-0 items-center gap-1">
                          <Button
                            variant="ghost"
                            size="sm"
                            aria-label={`Edit ${agent.name}`}
                            onClick={() => {
                              setEditing(agent);
                              setFormOpen(true);
                            }}
                          >
                            <Pencil className="h-3.5 w-3.5" />
                            <span className="hidden sm:inline">
                              {t("common.edit", { defaultValue: "Edit" })}
                            </span>
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            aria-label={`Delete ${agent.name}`}
                            className="text-destructive hover:text-destructive"
                            onClick={() => {
                              if (workflowUsesSubagent(agent.name)) {
                                toast.warning(
                                  t("agents.subagentDeleteBlockedByWorkflow", {
                                    defaultValue:
                                      "This subagent is selected in the Workflow Skill. Deselect it before deleting.",
                                  }),
                                );
                                return;
                              }
                              setConfirmDelete(agent);
                            }}
                            disabled={!agent.managed}
                            title={
                              agent.managed
                                ? undefined
                                : t("agents.unmanagedDeleteHint", {
                                    defaultValue:
                                      "Edit and save this existing agent to adopt it before removal.",
                                  })
                            }
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                            <span className="hidden sm:inline">
                              {t("common.delete", { defaultValue: "Delete" })}
                            </span>
                          </Button>
                        </div>
                      </li>
                    ))}
                  </ul>
                )}
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="workflow" className="mt-3">
            <Card className="rounded-xl border-border shadow-sm">
              <CardHeader className="px-4 py-3">
                <div className="flex items-center gap-2">
                  <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                    <FileText className="h-3.5 w-3.5" />
                  </span>
                  <div className="min-w-0">
                    <h2 className="text-sm font-semibold">
                      {t("agents.instructionsStage", {
                        defaultValue: "Workflow Skill",
                      })}
                    </h2>
                    <p className="text-xs text-muted-foreground">
                      {t("agents.instructionsStageDesc", {
                        defaultValue:
                          "Generates and installs the Workflow Skill at {{path}} so Codex can delegate bounded subtasks to your registered subagents.",
                        path:
                          status?.skillPath ??
                          "~/.codex/skills/cube-dispatch/SKILL.md",
                      })}
                    </p>
                  </div>
                </div>
              </CardHeader>
              <CardContent className="flex flex-col gap-3 px-4 pb-4 pt-0">
                <div className="grid gap-3 lg:grid-cols-3">
                  {(
                    [
                      ["worker", "agents.workflowRoleWorker", "Worker role"],
                      [
                        "explorer",
                        "agents.workflowRoleExplorer",
                        "Explorer role",
                      ],
                      ["default", "agents.workflowRoleDefault", "Default role"],
                    ] as const
                  ).map(([role, labelKey, labelDefault]) => (
                    <Field
                      key={role}
                      label={t(labelKey, { defaultValue: labelDefault })}
                      hint={t("agents.workflowRoleHint", {
                        defaultValue:
                          "Pick which registered subagents serve this role. One subagent may serve multiple roles; the first listed wins.",
                      })}
                    >
                      <div className="grid gap-1 rounded-md border p-2 sm:grid-cols-2">
                        {managedSubagents.map((agent) => (
                          <label
                            key={agent.name}
                            className="flex items-center gap-2 text-sm"
                          >
                            <input
                              type="checkbox"
                              checked={roleAgents[role].includes(agent.name)}
                              onChange={(event) =>
                                void toggleRole(
                                  role,
                                  agent.name,
                                  event.target.checked,
                                )
                              }
                            />
                            <span>{agent.name}</span>
                          </label>
                        ))}
                      </div>
                      {role === "default" && (
                        <p className="mt-2 text-xs text-muted-foreground">
                          {t("agents.workflowDefaultWorkerNote", {
                            defaultValue: "Default worker: {{name}}",
                            name: selectedWorker?.name ?? "—",
                          })}
                        </p>
                      )}
                    </Field>
                  ))}
                </div>

                {status?.installed && (
                  <div className="rounded-lg bg-muted/30">
                    <div className="px-3 py-1 text-xs font-medium text-muted-foreground">
                      {t("agents.currentWorkerSummary", {
                        defaultValue: "Current worker",
                      })}
                    </div>
                    <div className="grid gap-x-4 sm:grid-cols-2">
                      <ConfigSummary
                        label={t("agents.worker", { defaultValue: "Worker" })}
                        value={status.workerAgent || "—"}
                      />
                      <ConfigSummary
                        label={t("agents.workerModel", {
                          defaultValue: "Worker model",
                        })}
                        value={status.workerModel || "—"}
                      />
                      <ConfigSummary
                        label={t("agents.reasoning", {
                          defaultValue: "Worker reasoning effort",
                        })}
                        value={status.workerReasoningEffort || "—"}
                      />
                      <ConfigSummary
                        label={t("agents.sandbox", {
                          defaultValue: "Sandbox mode",
                        })}
                        value={status.sandboxMode || "—"}
                      />
                    </div>
                  </div>
                )}

                {status?.installed && status.skillContent && (
                  <div className="rounded-lg bg-muted/30">
                    <div className="flex items-start justify-between gap-2 px-3 py-2">
                      <div className="min-w-0">
                        <div className="text-xs font-medium text-muted-foreground">
                          {t("agents.injectedRules", {
                            defaultValue: "Workflow Skill content (SKILL.md)",
                          })}
                        </div>
                      </div>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-6 shrink-0 px-2 text-xs"
                        onClick={() => setPreviewOpen((open) => !open)}
                      >
                        {previewOpen
                          ? t("agents.previewHide", {
                              defaultValue: "Hide preview",
                            })
                          : t("agents.previewShow", {
                              defaultValue: "Preview",
                            })}
                      </Button>
                    </div>
                    {previewOpen && (
                      <pre className="mx-3 mb-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-md border border-border bg-background p-2 text-xs leading-relaxed text-muted-foreground">
                        {status.skillContent}
                      </pre>
                    )}
                  </div>
                )}

                <div
                  className="truncate rounded-lg bg-muted/30 px-3 py-1.5 text-xs text-muted-foreground"
                  title={status?.skillPath}
                >
                  {status?.skillPath ??
                    "~/.codex/skills/cube-dispatch/SKILL.md"}
                </div>

                {managedSubagents.length === 0 && (
                  <p className="text-xs text-muted-foreground">
                    {t("agents.noRegisteredWorkers", {
                      defaultValue:
                        "No registered subagents — register one above first.",
                    })}
                  </p>
                )}

                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    onClick={() => void installWorkflow()}
                    disabled={
                      busy ||
                      !selectedWorker ||
                      roleUnion(roleAgents).length === 0
                    }
                  >
                    {installing && <Loader2 className="h-4 w-4 animate-spin" />}
                    {installing
                      ? t("agents.installing", {
                          defaultValue: "Installing...",
                        })
                      : status?.installed
                        ? t("agents.update", {
                            defaultValue: "Update workflow skill",
                          })
                        : t("agents.workflowInstall", {
                            defaultValue: "Install workflow skill",
                          })}
                  </Button>
                  <Button
                    variant="outline"
                    onClick={() => setConfirmCancel(true)}
                    disabled={busy || !status?.canUndo}
                  >
                    {cancelling ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Undo2 className="h-3.5 w-3.5" />
                    )}
                    {cancelling
                      ? t("agents.cancellingInstructions", {
                          defaultValue: "Uninstalling…",
                        })
                      : t("agents.cancelInstructions", {
                          defaultValue: "Uninstall Skill",
                        })}
                  </Button>
                </div>
              </CardContent>
            </Card>
          </TabsContent>
        </Tabs>
      </div>

      <ConfirmDialog
        isOpen={confirmCancel}
        confirmText={t("common.confirm", { defaultValue: "Confirm" })}
        cancelText={t("common.cancel", { defaultValue: "Cancel" })}
        title={t("agents.confirmCancelInstructionsTitle", {
          defaultValue: "Uninstall workflow skill?",
        })}
        message={t("agents.confirmCancelInstructionsMessage", {
          defaultValue:
            "The workflow skill will be uninstalled and AGENTS.md restored to its pre-install content immediately (your existing content is unaffected).",
        })}
        pending={cancelling}
        onConfirm={() => {
          setConfirmCancel(false);
          void cancelInstructions();
        }}
        onCancel={() => setConfirmCancel(false)}
      />

      <SubagentFormDialog
        open={formOpen}
        editing={editing}
        onClose={() => {
          setFormOpen(false);
          setEditing(null);
        }}
        onSaved={() => {
          setFormOpen(false);
          setEditing(null);
          void loadSubagents();
        }}
      />

      <ConfirmDialog
        isOpen={confirmDelete !== null}
        confirmText={t("common.confirm", { defaultValue: "Confirm" })}
        cancelText={t("common.cancel", { defaultValue: "Cancel" })}
        title={t("agents.confirmDeleteSubagentTitle", {
          defaultValue: "Delete subagent?",
        })}
        message={t("agents.confirmDeleteSubagentMessage", {
          defaultValue: "Delete the subagent {{name}}?",
          name: confirmDelete?.name ?? "",
        })}
        pending={deleting}
        onConfirm={() => void deleteSubagent()}
        onCancel={() => setConfirmDelete(null)}
      />
    </div>
  );
}

/** 角色 → 默认 worker：default 角色第一个 → worker 角色第一个 → explorer 角色第一个 → 空。 */
function deriveWorkerName(roleAgents: CodexAgentWorkflowRoleAgents): string {
  return (
    roleAgents.default[0] ??
    roleAgents.worker[0] ??
    roleAgents.explorer[0] ??
    ""
  );
}

/** 全部角色选中的 agent 并集（去重）。 */
function roleUnion(roleAgents: CodexAgentWorkflowRoleAgents): string[] {
  return Array.from(
    new Set([
      ...roleAgents.default,
      ...roleAgents.worker,
      ...roleAgents.explorer,
    ]),
  );
}

function agentTypeLabel(
  agentType: string,
  t: (key: string, options?: { defaultValue?: string }) => string,
): string {
  if (agentType === "explorer") {
    return t("agents.roleExplorer", { defaultValue: "Explorer" });
  }
  if (agentType === "default") {
    return t("agents.roleDefault", { defaultValue: "Default" });
  }
  return t("agents.roleWorker", { defaultValue: "Worker" });
}

function sandboxLabel(
  mode: CodexSubagentSandboxMode,
  t: (key: string, options?: { defaultValue?: string }) => string,
): string {
  if (mode === "inherit") {
    return t("agents.sandboxInherit", {
      defaultValue: "Inherit parent agent / parent conversation",
    });
  }
  return mode;
}

/**
 * 与 Codex 内置/运行时 provider 冲突的 ID，避免子 agent provider 覆盖它们。
 * 与 src-tauri/src/codex_config.rs 的 CODEX_RESERVED_MODEL_PROVIDER_IDS 保持一致，
 * 并追加 Codex 自定义 provider 运行时 ID `custom`。
 */
const RESERVED_SUBAGENT_PROVIDER_IDS = new Set([
  "amazon-bedrock",
  "openai",
  "ollama",
  "lmstudio",
  "oss",
  "ollama-chat",
  "custom",
]);

/** 从 agent 名称推导 Cube 内部 ID；Codex 运行时 provider 固定为 custom。 */
export function deriveSubagentProviderId(name: string): string {
  const base =
    name
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9_-]+/g, "-")
      .replace(/-+/g, "-")
      .replace(/^-|-$/g, "") || "subagent";
  return RESERVED_SUBAGENT_PROVIDER_IDS.has(base) ? `${base}-worker` : base;
}

/** 从 Provider 的 settingsConfig 提取已存模型列表（catalog / config / 顶层字段）。 */
function storedModelsForProvider(provider: Provider): string[] {
  const config = provider.settingsConfig as Record<string, any>;
  const fromCatalog = Array.isArray(config.modelCatalog?.models)
    ? config.modelCatalog.models
        .map((m: any) => (typeof m?.model === "string" ? m.model.trim() : ""))
        .filter(Boolean)
    : [];
  const fromConfig = extractCodexModelName(
    typeof config.config === "string" ? config.config : "",
  );
  const fromTop = typeof config.model === "string" ? config.model.trim() : "";
  return Array.from(
    new Set([...fromCatalog, fromConfig, fromTop].filter(Boolean)),
  );
}

interface ReusableCodexProvider {
  id: string;
  name: string;
  baseUrl: string;
  apiKey: string;
  storedModels: string[];
  wireApi: CodexMemberWireApi;
}

/** 从 model 推导安全 agent 名称（[A-Za-z0-9][A-Za-z0-9_-]{0,63}）；推导失败返回空串。 */
export function slugifyAgentName(model: string): string {
  const slug = model
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^[^a-z0-9]+/, "")
    .replace(/-+$/, "")
    .slice(0, 64);
  return slug || "";
}

function SubagentFormDialog({
  open,
  editing,
  onClose,
  onSaved,
}: {
  open: boolean;
  editing: CodexSubagent | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [codexProviders, setCodexProviders] = useState<
    Record<string, Provider>
  >({});
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    providersApi
      .getAll("codex")
      .then((providers) => {
        if (!cancelled) setCodexProviders(providers);
      })
      .catch((error) => {
        console.error("[AgentsPanel] Failed to load Codex providers", error);
      });
    return () => {
      cancelled = true;
    };
  }, [open]);
  const reusableProviders = useMemo<ReusableCodexProvider[]>(() => {
    const providers = codexProviders;
    const result: ReusableCodexProvider[] = [];
    for (const provider of Object.values(providers)) {
      const { baseUrl, apiKey } = getCodexMemberCredentials(provider);
      if (!baseUrl || !apiKey) continue;
      // 自定义 subagent 仅支持 Responses：Chat/Anthropic 供应商不能复用。
      if (getCodexMemberWireApi(provider) !== "responses") continue;
      result.push({
        id: provider.id,
        name: provider.name.trim() || provider.id,
        baseUrl,
        apiKey,
        storedModels: storedModelsForProvider(provider),
        wireApi: getCodexMemberWireApi(provider),
      });
    }
    return result.sort((a, b) => a.name.localeCompare(b.name));
  }, [codexProviders]);

  const [source, setSource] = useState<"reuse" | "external">("reuse");
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [name, setName] = useState("");
  const [agentType, setAgentType] = useState("worker");
  const [model, setModel] = useState("");
  const [modelProviderId, setModelProviderId] = useState("");
  const [modelBaseUrl, setModelBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  /** 复用 Provider 时从其配置读取的 key：仅用于拉模型，不进入保存 payload。 */
  const [reuseApiKey, setReuseApiKey] = useState("");
  const [reasoningEffort, setReasoningEffort] = useState("high");
  const [modelCandidates, setModelCandidates] = useState<
    CodexSubagentModelCandidate[]
  >([]);
  const [loadingModels, setLoadingModels] = useState(false);
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState("");

  useEffect(() => {
    if (!open) return;
    setSource("reuse");
    setSelectedProviderId("");
    setApiKey("");
    setReuseApiKey("");
    setModelCandidates([]);
    setFormError("");
    if (editing) {
      setName(editing.name);
      setAgentType(
        AGENT_TYPES.includes(editing.agentType as AgentType)
          ? editing.agentType
          : "worker",
      );
      setModel(editing.model);
      setModelProviderId(editing.modelProviderId);
      setModelBaseUrl(editing.modelBaseUrl);
      setReasoningEffort(
        REASONING_EFFORTS.includes(editing.reasoningEffort)
          ? editing.reasoningEffort
          : "high",
      );
    } else {
      setName("");
      setAgentType("worker");
      setModel("");
      setModelProviderId("");
      setModelBaseUrl("");
      setReasoningEffort("high");
    }
  }, [open, editing]);

  const selectedProvider = useMemo(
    () => reusableProviders.find((p) => p.id === selectedProviderId) ?? null,
    [reusableProviders, selectedProviderId],
  );

  const handleSelectProvider = (providerId: string) => {
    const provider = reusableProviders.find((p) => p.id === providerId);
    if (!provider) return;
    setSelectedProviderId(providerId);
    setModelBaseUrl(provider.baseUrl);
    setReuseApiKey(provider.apiKey);
    setModelCandidates(
      provider.storedModels.map((m) => ({ model: m, displayName: null })),
    );
    const firstModel = provider.storedModels[0] ?? "";
    setModel(firstModel);
    // 复用供应商时按模型名自动预填名称；名称始终跟随当前模型。
    if (firstModel) {
      setName(slugifyAgentName(firstModel));
    }
  };

  const handleExternalUrlChange = (url: string) => {
    setModelBaseUrl(url);
  };

  const handleModelChange = (value: string) => {
    setModel(value);
    // 新建时按模型名自动预填名称（复用供应商与外部导入一致）。
    if (editing) return;
    const trimmed = value.trim();
    if (!trimmed) return;
    setName(slugifyAgentName(trimmed));
  };

  const redactApiKey = (detail: string) => {
    let out = detail;
    for (const key of [apiKey.trim(), reuseApiKey.trim()]) {
      if (key) out = out.split(key).join("[REDACTED]");
    }
    return out;
  };

  const loadModels = async () => {
    const providerId = editing
      ? modelProviderId
      : deriveSubagentProviderId(name);
    if (!providerId || !modelBaseUrl.trim()) {
      setFormError(
        t("agents.modelsNeedUrlAndKey", {
          defaultValue:
            "Name and base URL required; key needed only if none stored.",
        }),
      );
      return;
    }
    const key =
      editing || source === "external" ? apiKey.trim() : reuseApiKey.trim();
    setLoadingModels(true);
    setFormError("");
    try {
      const candidates = await codexAgentWorkflowApi.fetchSubagentModels({
        modelProviderId: providerId,
        modelBaseUrl: modelBaseUrl.trim(),
        apiKey: key,
      });
      setModelCandidates((prev) => {
        const merged = new Map<string, string | null>();
        for (const c of prev) merged.set(c.model, c.displayName);
        for (const c of candidates) merged.set(c.model, c.displayName);
        return Array.from(merged, ([m, displayName]) => ({
          model: m,
          displayName,
        }));
      });
      if (candidates.length === 0) {
        setFormError(
          t("agents.modelsEmpty", {
            defaultValue: "No models returned — type one manually",
          }),
        );
      }
    } catch (error) {
      const detail = redactApiKey(extractErrorMessage(error));
      console.error("[AgentsPanel] Failed to fetch subagent models", detail);
      const prefix = t("agents.modelsLoadFailed", {
        defaultValue:
          "Failed to fetch models — you can still type one manually",
      });
      setFormError(detail ? `${prefix}: ${detail}` : prefix);
    } finally {
      setLoadingModels(false);
    }
  };

  const save = async () => {
    const trimmedName = name.trim();
    const providerId = editing
      ? modelProviderId
      : deriveSubagentProviderId(trimmedName);
    const keyForSave =
      !editing && source === "reuse" ? reuseApiKey.trim() : apiKey.trim();
    if (!trimmedName || !model.trim() || !providerId || !modelBaseUrl.trim()) {
      setFormError(
        t("agents.requiredFieldsMissing", {
          defaultValue: "Name, model and base URL are required",
        }),
      );
      return;
    }
    if (!editing && !keyForSave) {
      setFormError(
        t("agents.apiKeyRequired", {
          defaultValue: "API key is required for a new subagent",
        }),
      );
      return;
    }
    setSaving(true);
    setFormError("");
    try {
      await codexAgentWorkflowApi.upsertSubagent({
        name: trimmedName,
        // 描述由后端按类型自动生成（编辑时保留现有描述），表单不再要求输入。
        description: "",
        agentType,
        model: model.trim(),
        modelProviderId: providerId,
        modelBaseUrl: modelBaseUrl.trim(),
        ...(keyForSave ? { apiKey: keyForSave } : {}),
        sandboxMode: "inherit",
        reasoningEffort,
        // 自定义 subagent 仅支持 Responses 协议。
        wireApi: "responses",
      });
      toast.success(
        t("agents.subagentSaved", { defaultValue: "Subagent saved" }),
      );
      onSaved();
    } catch (error) {
      const detail = redactApiKey(extractErrorMessage(error));
      console.error("[AgentsPanel] Failed to save subagent", detail);
      const prefix = t("agents.subagentSaveFailed", {
        defaultValue: "Failed to save subagent",
      });
      setFormError(detail ? `${prefix}: ${detail}` : prefix);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && !saving) onClose();
      }}
    >
      <DialogContent
        className="w-[min(34rem,calc(100vw-2rem))] gap-0 overflow-hidden"
        zIndex="alert"
      >
        <DialogHeader>
          <DialogTitle>
            {editing
              ? t("agents.editSubagent", { defaultValue: "Edit subagent" })
              : t("agents.addSubagent", { defaultValue: "Add subagent" })}
          </DialogTitle>
        </DialogHeader>
        <div className="grid min-h-0 min-w-0 flex-1 gap-3 overflow-y-auto px-6 pb-2 grid-cols-[minmax(0,1fr)] sm:grid-cols-[repeat(2,minmax(0,1fr))]">
          {!editing && (
            <div className="min-w-0 sm:col-span-2">
              <Tabs
                value={source}
                onValueChange={(value) =>
                  setSource(value as "reuse" | "external")
                }
              >
                <TabsList className="w-full">
                  <TabsTrigger value="reuse">
                    {t("agents.formSourceReuse", {
                      defaultValue: "Reuse existing Codex provider",
                    })}
                  </TabsTrigger>
                  <TabsTrigger value="external">
                    {t("agents.formSourceExternal", {
                      defaultValue: "External import",
                    })}
                  </TabsTrigger>
                </TabsList>
                {source === "reuse" && (
                  <TabsContent value="reuse" className="mt-2 space-y-3">
                    <Field
                      label={t("agents.formProvider", {
                        defaultValue: "Codex provider",
                      })}
                    >
                      <Select
                        value={selectedProviderId}
                        onValueChange={handleSelectProvider}
                      >
                        <SelectTrigger
                          className="h-8"
                          aria-label={t("agents.formProvider", {
                            defaultValue: "Codex provider",
                          })}
                        >
                          <SelectValue
                            placeholder={t("agents.formProviderPlaceholder", {
                              defaultValue: "Select a provider…",
                            })}
                          />
                        </SelectTrigger>
                        <SelectContent>
                          {reusableProviders.length === 0 && (
                            <p className="px-3 py-2 text-xs text-muted-foreground">
                              {t("agents.noReusableProviders", {
                                defaultValue:
                                  "No reusable providers — use External import instead.",
                              })}
                            </p>
                          )}
                          {reusableProviders.map((provider) => (
                            <SelectItem key={provider.id} value={provider.id}>
                              {provider.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </Field>
                    {selectedProvider && (
                      <div className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
                        <p className="font-medium text-foreground">
                          {selectedProvider.name}
                        </p>
                        <p className="mt-0.5 break-all">
                          {selectedProvider.baseUrl}
                        </p>
                      </div>
                    )}
                  </TabsContent>
                )}
                {source === "external" && (
                  <TabsContent value="external" className="mt-2 space-y-3">
                    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                      <Field
                        id="subagent-base-url"
                        label={t("agents.formModelBaseUrl", {
                          defaultValue: "Model base URL",
                        })}
                      >
                        <Input
                          id="subagent-base-url"
                          value={modelBaseUrl}
                          onChange={(event) =>
                            handleExternalUrlChange(event.target.value)
                          }
                          placeholder={t("agents.formModelBaseUrlPlaceholder", {
                            defaultValue: "e.g. https://api.deepseek.com",
                          })}
                          className="h-8"
                        />
                      </Field>
                      <ApiKeyInput
                        value={apiKey}
                        onChange={setApiKey}
                        required
                        label={t("agents.formApiKey", {
                          defaultValue: "API key",
                        })}
                        placeholder={t("agents.apiKeyCreatePlaceholder", {
                          defaultValue: "Required to call the provider API",
                        })}
                      />
                    </div>
                  </TabsContent>
                )}
              </Tabs>
            </div>
          )}
          {editing && (
            <div className="min-w-0 sm:col-span-2">
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <Field
                  id="subagent-base-url"
                  label={t("agents.formModelBaseUrl", {
                    defaultValue: "Model base URL",
                  })}
                >
                  <Input
                    id="subagent-base-url"
                    value={modelBaseUrl}
                    onChange={(event) => setModelBaseUrl(event.target.value)}
                    placeholder={t("agents.formModelBaseUrlPlaceholder", {
                      defaultValue: "e.g. https://api.deepseek.com",
                    })}
                    className="h-8"
                  />
                </Field>
                <ApiKeyInput
                  value={apiKey}
                  onChange={setApiKey}
                  label={t("agents.formApiKey", { defaultValue: "API key" })}
                  placeholder={t("agents.apiKeyKeepPlaceholder", {
                    defaultValue: "Leave blank to keep the current key",
                  })}
                />
              </div>
            </div>
          )}
          <div className="min-w-0 sm:col-span-2">
            <Field
              id="subagent-model"
              label={t("agents.formModel", { defaultValue: "Model" })}
            >
              <div className="flex flex-wrap items-start gap-2">
                <div className="min-w-0 flex-1">
                  <ModelCombobox
                    id="subagent-model"
                    value={model}
                    candidates={modelCandidates}
                    onChange={handleModelChange}
                    ariaLabel={t("agents.formModel", {
                      defaultValue: "Model",
                    })}
                    placeholder={t("agents.formModelPlaceholder", {
                      defaultValue: "e.g. deepseek-v4-flash",
                    })}
                  />
                </div>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-8 shrink-0"
                  onClick={() => void loadModels()}
                  disabled={loadingModels}
                >
                  {loadingModels ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <RefreshCw className="h-3.5 w-3.5" />
                  )}
                  {loadingModels
                    ? t("agents.loadingModels", {
                        defaultValue: "Loading models...",
                      })
                    : t("agents.loadModels", {
                        defaultValue: "Load models",
                      })}
                </Button>
              </div>
            </Field>
          </div>
          <Field
            label={t("agents.reasoning", {
              defaultValue: "Worker reasoning effort",
            })}
          >
            <Select value={reasoningEffort} onValueChange={setReasoningEffort}>
              <SelectTrigger
                className="h-8"
                aria-label={t("agents.reasoning", {
                  defaultValue: "Worker reasoning effort",
                })}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {REASONING_EFFORTS.map((effort) => (
                  <SelectItem key={effort} value={effort}>
                    {effort}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
          <div className="min-w-0 sm:col-span-2">
            <Field
              id="subagent-agent-type"
              label={t("agents.formAgentType", {
                defaultValue: "Role type",
              })}
              hint={t("agents.formAgentTypeHint", {
                defaultValue:
                  "Description and developer instructions are generated automatically from the role type. The Workflow Skill dispatches by role.",
              })}
            >
              <Select value={agentType} onValueChange={setAgentType}>
                <SelectTrigger
                  className="h-8"
                  aria-label={t("agents.formAgentType", {
                    defaultValue: "Role type",
                  })}
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {AGENT_TYPES.map((type) => (
                    <SelectItem key={type} value={type}>
                      {agentTypeLabel(type, t)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
          </div>
        </div>
        {formError && (
          <p className="max-h-24 shrink-0 overflow-y-auto break-words px-6 pb-2 text-xs font-medium text-destructive">
            {formError}
          </p>
        )}
        <DialogFooter className="flex gap-2 sm:justify-end">
          <Button variant="outline" onClick={onClose} disabled={saving}>
            {t("common.cancel", { defaultValue: "Cancel" })}
          </Button>
          <Button onClick={() => void save()} disabled={saving}>
            {saving && <Loader2 className="h-4 w-4 animate-spin" />}
            {saving
              ? t("common.saving", { defaultValue: "Saving..." })
              : t("common.save", { defaultValue: "Save" })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Field({
  id,
  label,
  hint,
  children,
}: {
  id?: string;
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="min-w-0">
      <Label htmlFor={id} className="text-xs font-medium text-muted-foreground">
        {label}
      </Label>
      <div className="mt-1">{children}</div>
      {hint && (
        <p className="mt-1 text-xs text-muted-foreground" title={hint}>
          {hint}
        </p>
      )}
    </div>
  );
}

function ConfigSummary({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex min-w-0 items-center gap-2 px-3 py-1.5">
      <span
        className="w-36 shrink-0 truncate text-xs text-muted-foreground"
        title={label}
      >
        {label}
      </span>
      <span
        className="min-w-0 flex-1 truncate text-sm font-medium"
        title={value}
      >
        {value}
      </span>
    </div>
  );
}

function ModelCombobox({
  id,
  value,
  candidates,
  onChange,
  placeholder,
  ariaLabel,
}: {
  id?: string;
  value: string;
  candidates: CodexSubagentModelCandidate[];
  onChange: (value: string) => void;
  placeholder?: string;
  ariaLabel?: string;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");

  const selected = useMemo(
    () => candidates.find((candidate) => candidate.model === value),
    [candidates, value],
  );
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return candidates;
    return candidates.filter((candidate) =>
      [candidate.model, candidate.displayName]
        .filter(Boolean)
        .some((part) => part!.toLowerCase().includes(q)),
    );
  }, [candidates, query]);

  const customQuery =
    query.trim() && !candidates.some((c) => c.model === query.trim());

  return (
    <Popover
      modal
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) setQuery("");
      }}
    >
      <PopoverTrigger asChild>
        <button
          id={id}
          type="button"
          role="combobox"
          aria-label={ariaLabel}
          aria-haspopup="listbox"
          aria-expanded={open}
          className="flex h-8 w-full items-center justify-between gap-2 rounded-md border border-input bg-background px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
        >
          <span
            className="min-w-0 flex-1 truncate text-left"
            title={
              selected
                ? candidateLabel(selected)
                : value ||
                  placeholder ||
                  t("agents.selectModel", {
                    defaultValue: "Select a model…",
                  })
            }
          >
            {selected
              ? candidateLabel(selected)
              : value ||
                placeholder ||
                t("agents.selectModel", { defaultValue: "Select a model…" })}
          </span>
          {!selected && value && (
            <span className="shrink-0 rounded-full border border-border px-1.5 py-px text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
              {t("agents.customChip", { defaultValue: "Custom" })}
            </span>
          )}
          <ChevronsUpDown className="h-4 w-4 shrink-0 opacity-50" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="bottom"
        align="start"
        sideOffset={6}
        className="z-[1000] w-[var(--radix-popover-trigger-width)] p-0"
      >
        <Command>
          <CommandInput
            value={query}
            onValueChange={setQuery}
            onKeyDown={(event) => {
              if (event.key === "Enter" && customQuery) {
                event.preventDefault();
                onChange(query.trim());
                setOpen(false);
              }
            }}
            placeholder={t("agents.searchModel", {
              defaultValue: "Search models…",
            })}
          />
          <CommandList>
            <CommandEmpty>
              {candidates.length === 0
                ? t("agents.noModelCandidates", {
                    defaultValue:
                      "No models available — type to use a custom model",
                  })
                : t("agents.noModels", {
                    defaultValue: "No matching models",
                  })}
            </CommandEmpty>
            <CommandGroup>
              {filtered.map((candidate) => (
                <CommandItem
                  key={candidate.model}
                  value={candidate.model}
                  onSelect={() => {
                    onChange(candidate.model);
                    setOpen(false);
                  }}
                >
                  <span className="min-w-0 flex-1 truncate">
                    {candidateLabel(candidate)}
                  </span>
                </CommandItem>
              ))}
              {customQuery && (
                <CommandItem
                  value={query.trim()}
                  onSelect={() => {
                    onChange(query.trim());
                    setOpen(false);
                  }}
                >
                  <Plus className="mr-2 h-3.5 w-3.5 opacity-50" />
                  <span className="min-w-0 flex-1 truncate">
                    {t("agents.useCustomModel", {
                      defaultValue: "Use custom model: {{model}}",
                      model: query.trim(),
                    })}
                  </span>
                </CommandItem>
              )}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

function candidateLabel(candidate: CodexSubagentModelCandidate): string {
  return candidate.displayName
    ? `${candidate.model} · ${candidate.displayName}`
    : candidate.model;
}
