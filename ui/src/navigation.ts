export function toHistoryPath(uiUrl: string, baseUrl: string): string {
  const url = new URL(uiUrl, baseUrl);
  return `${url.pathname}${url.search}`;
}
