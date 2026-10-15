# Contribuer à Murmure

Merci de votre intérêt. Les corrections de bugs, les retours sur une distribution Linux ou un bureau
particulier et les améliorations de la documentation sont les contributions les plus utiles.

## Avant de commencer

Pour un bug, ouvrez d'abord une issue avec le modèle « Signaler un bug » : les commandes de diagnostic
qu'il contient font souvent gagner beaucoup de temps. Pour une nouvelle fonctionnalité ou un changement
important, ouvrez une issue avant d'écrire le code, pour vérifier que la direction convient.

Une faille de sécurité ne se signale jamais dans une issue publique : voir [SECURITY.md](SECURITY.md).

## Préparer son environnement

Il faut Rust stable, Node.js 22 ou plus récent, pnpm 11 et CMake. Pour le serveur, il faut aussi
[uv](https://docs.astral.sh/uv/). Sous Linux (Ubuntu 22.04 ou équivalent), installez :

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf \
  libasound2-dev libxdo-dev libssl-dev cmake clang
```

Puis, à la racine du dépôt :

```bash
pnpm install
pnpm tauri dev
```

Le détail (variantes GPU, tests sur un vrai modèle, mode démo du front) est dans
[docs/developpement.md](docs/developpement.md). L'organisation du code est décrite dans
[ARCHITECTURE.md](ARCHITECTURE.md).

## Vérifications avant une pull request

La CI lance les mêmes commandes sur macOS, Linux et Windows. Lancez au moins celles qui concernent votre
changement :

```bash
pnpm build                                                  # TypeScript et front
(cd src-tauri && cargo test --lib)                          # tests Rust
(cd src-tauri && cargo clippy --all-targets -- -D warnings)
(cd server && uv run pytest)                                # serveur, si vous l'avez modifié
```

## Conventions

- L'interface est entièrement en français, avec la typographie française (« », espaces avant « : »).
  Les commentaires du code et les messages de commit aussi.
- Messages de commit courts, au présent, préfixés si possible : `fix(linux): …`, `feat: …`, `docs: …`.
- Une pull request traite un seul sujet. Ne reformatez pas des fichiers entiers : le code n'est pas
  passé à `cargo fmt` et un reformatage global rend la relecture impossible.
- Toute logique testable sans matériel (conversion, décision, analyse de texte) a son test unitaire, à
  côté du code, comme dans `hotkey.rs` ou `punctuation.rs`.
- Une nouvelle dépendance doit avoir une licence compatible avec l'AGPL v3 (MIT, Apache-2.0, BSD,
  MPL-2.0…). Ajoutez-la au fichier `NOTICE` si elle est importante.
- Les fichiers source commencent par l'en-tête `SPDX-License-Identifier: AGPL-3.0-or-later`.

## Accord de licence de contribution

Chaque contribution est soumise à l'accord de licence de contribution décrit dans [CLA.md](CLA.md). Vous
restez propriétaire de votre code, mais vous accordez à RD Agency une licence large, qui lui permet entre
autres de proposer Murmure sous d'autres licences, y compris commerciales. Le modèle de pull request
contient une case à cocher : une pull request sans cette case cochée ne sera pas fusionnée.

## Ce qui se passe ensuite

Pour une première contribution, les workflows GitHub Actions attendent l'approbation d'un mainteneur avant
de s'exécuter. C'est une protection normale sur un dépôt public. La CI des pull requests tourne uniquement
sur les machines hébergées par GitHub, sans accès aux secrets du dépôt. Les builds qui demandent un GPU et
la publication des versions sont lancés par les mainteneurs après la fusion.

## Code de conduite

La participation au projet suit le [code de conduite](CODE_OF_CONDUCT.md).
