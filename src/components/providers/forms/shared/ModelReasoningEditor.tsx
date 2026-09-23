import { useTranslation } from "react-i18next";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { CODEX_REASONING_EFFORTS } from "@/utils/aggregateProvider";

export interface ModelReasoningValue {
  reasoningEfforts?: string[];
  defaultReasoningEffort?: string;
}

const ALL_EFFORTS = [...CODEX_REASONING_EFFORTS] as string[];

/** 全选/默认 = 六档（与后端 normalize 默认一致，未设置即全部档位）。 */
function normalizeSubset(value?: string[]): string[] | undefined {
  if (!value || value.length === 0) return undefined;
  const filtered = ALL_EFFORTS.filter((effort) => value.includes(effort));
  if (filtered.length === 0 || filtered.length === ALL_EFFORTS.length) {
    return undefined;
  }
  return filtered;
}

export function ModelReasoningEditor({
  value,
  onChange,
  compact,
}: {
  value: ModelReasoningValue;
  onChange: (next: ModelReasoningValue) => void;
  compact?: boolean;
}) {
  const { t } = useTranslation();
  const subset = normalizeSubset(value.reasoningEfforts);
  const selected = new Set(subset ?? ALL_EFFORTS);
  const defaultEffort = value.defaultReasoningEffort;
  const effectiveDefault =
    defaultEffort && selected.has(defaultEffort) ? defaultEffort : undefined;
  const label = !subset
    ? t("codexConfig.reasoningAllLevels", { defaultValue: "全部档位" })
    : subset.join(" / ");
  const defaultLabel = effectiveDefault
    ? t("codexConfig.reasoningDefaultShort", {
        defaultValue: "默认 {{effort}}",
        effort: effectiveDefault,
      })
    : "";

  const toggle = (effort: string, checked: boolean) => {
    const next = new Set(selected);
    if (checked) next.add(effort);
    else next.delete(effort);
    if (next.size === 0) return;
    const ordered = ALL_EFFORTS.filter((item) => next.has(item));
    const nextSubset =
      ordered.length === ALL_EFFORTS.length ? undefined : ordered;
    // 默认档若被排除出子集则跟随清空（后端也会钳制，界面保持一致）。
    const nextDefault =
      value.defaultReasoningEffort && next.has(value.defaultReasoningEffort)
        ? value.defaultReasoningEffort
        : undefined;
    onChange({
      reasoningEfforts: nextSubset,
      defaultReasoningEffort: nextDefault,
    });
  };

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className={
            compact
              ? "h-9 w-full justify-start truncate px-2 text-xs"
              : "h-9 w-full justify-start truncate text-xs"
          }
          title={[
            t("codexConfig.catalogColumnReasoning", {
              defaultValue: "思考档位",
            }),
            defaultLabel,
          ]
            .filter(Boolean)
            .join(" · ")}
        >
          <span className="truncate">
            {label}
            {defaultLabel ? ` · ${effectiveDefault}` : ""}
          </span>
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-56 space-y-3 p-3" align="start">
        <p className="text-xs text-muted-foreground">
          {t("codexConfig.reasoningLevelsHint", {
            defaultValue:
              "该模型在 Codex 菜单中可选的思考档位；全选即全部六档。",
          })}
        </p>
        <div className="space-y-1.5">
          {ALL_EFFORTS.map((effort) => (
            <label
              key={effort}
              className="flex cursor-pointer items-center gap-2 text-sm"
            >
              <Checkbox
                checked={selected.has(effort)}
                onCheckedChange={(checked) => toggle(effort, checked === true)}
              />
              <span className="font-mono">{effort}</span>
            </label>
          ))}
        </div>
        <div className="space-y-1.5 border-t border-border-default pt-2">
          <span className="text-xs text-muted-foreground">
            {t("codexConfig.reasoningDefaultLabel", {
              defaultValue: "默认档位",
            })}
          </span>
          <Select
            value={effectiveDefault ?? "__none"}
            onValueChange={(next) =>
              onChange({
                reasoningEfforts: subset,
                defaultReasoningEffort: next === "__none" ? undefined : next,
              })
            }
          >
            <SelectTrigger className="h-8 text-xs">
              <SelectValue
                placeholder={t("codexConfig.reasoningDefaultPlaceholder", {
                  defaultValue: "随全局",
                })}
              />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none">
                {t("codexConfig.reasoningDefaultPlaceholder", {
                  defaultValue: "随全局",
                })}
              </SelectItem>
              {[...selected].map((effort) => (
                <SelectItem key={effort} value={effort}>
                  {effort}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </PopoverContent>
    </Popover>
  );
}
