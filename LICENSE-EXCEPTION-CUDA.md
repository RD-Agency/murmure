# Murmure — exception de liaison NVIDIA CUDA

## Résumé en français

Murmure est distribué sous la licence GNU AGPL v3 ou ultérieure (fichier `LICENSE`). La variante Linux
NVIDIA de l'application embarque trois bibliothèques de NVIDIA (`libcudart`, `libcublas` et `libcublasLt`).
Ces bibliothèques sont propriétaires et distribuées sous le contrat de licence utilisateur final du
CUDA Toolkit. Sans autorisation particulière, l'AGPL interdirait de distribuer Murmure avec elles.

Le texte ci-dessous, rédigé en anglais pour sa valeur juridique, est une permission additionnelle au sens
de la section 7 de l'AGPL. Il autorise la distribution de Murmure liée à ces bibliothèques, sans obligation
d'en fournir le code source. Tout le reste de Murmure reste soumis à l'AGPL : le code de Murmure lui-même
doit toujours être fourni, y compris pour la variante NVIDIA.

Vous pouvez retirer cette exception de votre propre version modifiée, comme le permet la section 7.
Seul le texte anglais fait foi.

## Additional permission under GNU AGPL version 3 section 7

Copyright © 2026 RD Agency (Rayan Drissi)

This file states an additional permission to the GNU Affero General Public License version 3, or (at your
option) any later version, under which Murmure ("the Program") is licensed. It applies to every file of
the Program whose license notice refers to the GNU Affero General Public License.

**Definitions.** "CUDA Libraries" means the libraries of the NVIDIA CUDA Toolkit and of the NVIDIA cuDNN
library, as distributed by NVIDIA Corporation under the NVIDIA CUDA Toolkit End User License Agreement or
the NVIDIA cuDNN Software License Agreement, including without limitation `libcudart`, `libcublas`,
`libcublasLt` and `libcudnn`, and any later version of those libraries distributed by NVIDIA under
substantially similar terms.

**Permission.** If you modify the Program, or any covered work, by linking or combining it with the CUDA
Libraries (or a modified version of those libraries), containing parts covered by the terms of the
licenses under which NVIDIA distributes them, the licensors of the Program grant you additional permission
to convey the resulting work. Corresponding Source for a non-source form of such a combination shall not
include the source code for the parts of the CUDA Libraries used as well as that of the covered work.

**Scope.** This permission does not change the license of any part of the Program itself. The Corresponding
Source of the Program, including all modifications you make to it, must still be made available under the
terms of the GNU Affero General Public License. This permission does not grant you any right in the CUDA
Libraries; your use and redistribution of them remain subject to the terms set by NVIDIA.

**Removal.** As provided in section 7 of the GNU Affero General Public License, you may remove this
additional permission from your copy of the Program, or from any part of it, when you convey it. If you
modify the Program, you may extend this permission to your version, but you are not obligated to do so.
