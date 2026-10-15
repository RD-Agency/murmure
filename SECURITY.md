# Sécurité

## Signaler une faille

Ne signalez pas une faille de sécurité dans une issue publique.

Utilisez le signalement privé de GitHub : onglet **Security** du dépôt, puis **Report a vulnerability**
(lien direct : <https://github.com/RD-Agency/murmure/security/advisories/new>). Seuls les mainteneurs voient
le rapport. Si vous ne pouvez pas utiliser GitHub, écrivez à rayan.drissi@icloud.com.

Indiquez si possible :

- la version de Murmure et le système concernés ;
- la variante (standard, NVIDIA ou Vulkan) et le moteur (local ou serveur) ;
- les étapes pour reproduire le problème et son impact.

Nous accusons réception sous sept jours. Nous vous tenons informé de l'analyse, puis du correctif. La faille
est rendue publique, avec votre nom si vous le souhaitez, une fois la version corrigée publiée.

## Versions suivies

Seule la dernière version publiée reçoit des correctifs de sécurité. Les installations à jour la reçoivent
automatiquement, sauf les paquets Linux .deb et .rpm, qui affichent un bouton de mise à jour.

## Périmètre

Sont concernés l'application de bureau, le serveur de transcription (`server/`), la chaîne de publication
(workflows GitHub Actions, manifestes de mise à jour) et la page de téléchargement.

Points particulièrement sensibles :

- la vérification de la signature des mises à jour automatiques ;
- tout envoi d'audio ou de texte hors de la machine sans action explicite de l'utilisateur ;
- le stockage de la clé API (trousseau du système) ;
- l'authentification et les journaux du serveur, qui ne doivent jamais contenir l'audio ni le texte.
