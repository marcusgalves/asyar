import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../../services/log/logService', () => ({
  logService: { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

import { invoke } from '@tauri-apps/api/core';
import {
  ocrCaptureScreenText,
  ocrCaptureScreenTextForExtension,
  ocrClipboardItemText,
} from './ocrCommands';

describe('ocrCommands', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('ocrCaptureScreenText invokes ocr_capture_screen_text and returns recognized text', async () => {
    vi.mocked(invoke).mockResolvedValueOnce('Hello OCR');

    const result = await ocrCaptureScreenText();

    expect(invoke).toHaveBeenCalledWith('ocr_capture_screen_text', undefined);
    expect(result).toBe('Hello OCR');
  });

  it('ocrCaptureScreenTextForExtension invokes ocr_capture_screen_text via invokeRaw', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(null);

    const result = await ocrCaptureScreenTextForExtension();

    expect(invoke).toHaveBeenCalledWith('ocr_capture_screen_text', undefined);
    expect(result).toBeNull();
  });

  it('ocrClipboardItemText invokes ocr_clipboard_item_text with the item id', async () => {
    vi.mocked(invoke).mockResolvedValueOnce('Extracted from clipboard image');

    const result = await ocrClipboardItemText('clip-img-123');

    expect(invoke).toHaveBeenCalledWith('ocr_clipboard_item_text', { id: 'clip-img-123' });
    expect(result).toBe('Extracted from clipboard image');
  });

  it('ocrClipboardItemText rejects with IpcError when the backend command fails', async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error('not an image'));

    await expect(ocrClipboardItemText('clip-txt-1')).rejects.toMatchObject({
      name: 'IpcError',
      command: 'ocr_clipboard_item_text',
    });
  });
});
