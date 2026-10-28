# Répétition préliminaire

Ce premier run a effectivement passé le scénario local (état actuel `open`, livraison doublonnée, credential invalide classé `inconclusive`). Sa table de provenance, écrite par la première version du driver, ne couvrait pas tous les modules Rust compilés, le helper Python importé, le toolchain ni les exécutables construits. Il est conservé sans suppression comme trace de la première répétition, mais ne constitue pas la preuve de provenance de référence.

Le run `webhook-live-reconcile-002` a rejoué le même parcours avec ces éléments ajoutés à `provenance.json`; consulter son [`local-run.md`](../webhook-live-reconcile-002/local-run.md) et son manifeste SHA-256 pour la preuve complète disponible.
