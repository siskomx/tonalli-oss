// Adapted from Project NOMAD admin/inertia/components/chat/ChatAssistantAvatar.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: JSX conversion and explicit React import.
import React from 'react';
import { IconWand } from '@tabler/icons-react';
export default function ChatAssistantAvatar() {
  return <div className="flex-shrink-0"><div className="h-8 w-8 rounded-full bg-desert-green flex items-center justify-center">
    <IconWand className="h-5 w-5 text-white" aria-hidden="true" />
  </div></div>;
}
