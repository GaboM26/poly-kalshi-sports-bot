/**
 * Mention (Trump speeches) backend endpoints.
 */
import { apiUrl } from '../../shared/api';

/** URL of a mention route, e.g. `mentionUrl('/status')`. */
export function mentionUrl(path: string): string {
  return apiUrl(`/api/mention${path}`);
}
