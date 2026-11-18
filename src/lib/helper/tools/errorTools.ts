/** Readable message from any error (Error, Tauri command string, etc.), translating legacy backend strings into English. */
const FR_TO_EN: [string, string][] = [
  ["serveur injoignable :", "Server unreachable:"],
  ["identifiant, mot de passe ou jeton refusé", "Invalid username, password, or token"],
  ["identifiant et mot de passe requis", "Username and password are required"],
  ["nom d'utilisateur requis", "Username is required"],
  ["adresse du serveur vide", "Server address is empty"],
  ["adresse du serveur invalide", "Invalid server address"],
  ["ce n'est pas un serveur Subsonic", "Not a valid Subsonic server"],
  ["ce n'est pas un serveur Jellyfin", "Not a valid Jellyfin server"],
  ["ce n'est pas un serveur DLNA", "Not a valid DLNA server"],
  ["élément introuvable sur le serveur", "Item not found on server"],
  ["aucune bibliothèque choisie", "No library selected"],
  ["serveur supprimé", "Server removed"],
  ["serveur désactivé", "Server disabled"],
  ["serveur inconnu", "Unknown server"],
  ["identifiants du serveur manquants", "Missing server credentials"],
  ["aucun profil", "No active profile"],
  ["nom vide", "Name cannot be empty"],
];

export function errorMessage(e: unknown): string {
  let msg = "";
  if (e instanceof Error) msg = e.message;
  else if (typeof e === "object" && e && "message" in e) msg = String((e as { message: unknown }).message);
  else msg = String(e ?? "");

  for (const [fr, en] of FR_TO_EN) {
    if (msg.includes(fr)) {
      msg = msg.replace(fr, en);
    }
  }
  return msg;
}
