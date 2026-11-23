import { iosShell } from '../../app/platform.js';

// The iPhone app is a Bluetooth-only radio client: iOS gives apps no USB serial host.
export const transportsFor = (userAgent = navigator.userAgent) =>
  iosShell(userAgent) ? ['ble'] : ['ble', 'serial'];
