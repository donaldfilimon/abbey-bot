/**
 * Preferred rebuild path: import { DiscordSDK } from '@discord/embedded-app-sdk'
 * then discordSdk.ready(). This repo ships a same-origin bundle in ../app.js so
 * the Activity iframe does not need a second URL mapping for a CDN.
 */
import { DiscordSDK } from '@discord/embedded-app-sdk';

const CLIENT_ID = '1147940171099152464';
const status = document.getElementById('status');
const sdk = new DiscordSDK(CLIENT_ID);
await sdk.ready();
if (status) {
  status.textContent = 'Abbey is ready in this voice Activity.';
}
