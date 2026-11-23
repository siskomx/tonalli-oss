// Only the Tonalli iPhone app identifies itself this way; Safari and the other shells do not.
export const iosShell = (userAgent = navigator.userAgent) => /TonalliNative\/iOS/.test(userAgent);
