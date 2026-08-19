import React, { useState } from "react";
import { CodexAuthSection, CodexConfigSection } from "./CodexConfigSections";
import { CodexCommonConfigModal } from "./CodexCommonConfigModal";

interface CodexConfigEditorProps {
  authValue: string;

  configValue: string;

  providerName?: string;

  showRemoteCompaction?: boolean;

  isProxyTakeover?: boolean;

  onAuthChange: (value: string) => void;

  onConfigChange: (value: string) => void;

  onAuthBlur?: () => void;

  useCommonConfig: boolean;

  onCommonConfigToggle: (checked: boolean) => void | Promise<void>;

  commonConfigSnippet: string;

  onCommonConfigSnippetChange: (value: string) => boolean | Promise<boolean>;

  onCommonConfigErrorClear: () => void;

  commonConfigError: string;

  authError: string;

  configError: string; // config.toml 错误提示

  onExtract?: () => void;

  isExtracting?: boolean;

  showAuth?: boolean;
}

const CodexConfigEditor: React.FC<CodexConfigEditorProps> = ({
  authValue,
  configValue,
  providerName,
  showRemoteCompaction,
  isProxyTakeover = false,
  onAuthChange,
  onConfigChange,
  onAuthBlur,
  useCommonConfig,
  onCommonConfigToggle,
  commonConfigSnippet,
  onCommonConfigSnippetChange,
  onCommonConfigErrorClear,
  commonConfigError,
  authError,
  configError,
  onExtract,
  isExtracting,
  showAuth = true,
}) => {
  const [isCommonConfigModalOpen, setIsCommonConfigModalOpen] = useState(false);

  const handleCloseCommonConfigModal = () => {
    onCommonConfigErrorClear();
    setIsCommonConfigModalOpen(false);
  };

  return (
    <div className="space-y-6">
      {/* Auth JSON Section */}
      {showAuth && (
        <CodexAuthSection
          value={authValue}
          onChange={onAuthChange}
          onBlur={onAuthBlur}
          error={authError}
          isProxyTakeover={isProxyTakeover}
        />
      )}

      {/* Config TOML Section */}
      <CodexConfigSection
        value={configValue}
        onChange={onConfigChange}
        providerName={providerName}
        showRemoteCompaction={showRemoteCompaction}
        useCommonConfig={useCommonConfig}
        onCommonConfigToggle={onCommonConfigToggle}
        onEditCommonConfig={() => setIsCommonConfigModalOpen(true)}
        commonConfigError={commonConfigError}
        configError={configError}
        isProxyTakeover={isProxyTakeover}
      />

      {/* Common Config Modal */}
      <CodexCommonConfigModal
        isOpen={isCommonConfigModalOpen}
        onClose={handleCloseCommonConfigModal}
        value={commonConfigSnippet}
        onSave={onCommonConfigSnippetChange}
        error={commonConfigError}
        onExtract={onExtract}
        isExtracting={isExtracting}
      />
    </div>
  );
};

export default CodexConfigEditor;
