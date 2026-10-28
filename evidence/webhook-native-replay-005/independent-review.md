# Revue indépendante de WEBHOOK-NATIVE-REPLAY-005

La revue statique en lecture seule n'a relevé aucun défaut matériel dans les reçus : la tâche inactive est finalisée (et non simplement en vol), son état est `is_delivered=true`, `is_succeed=false`, statut interne 0, le hook est relu inactif, les compteurs proxy/journal ne progressent pas et le nettoyage confirme l'absence de listeners. Les hashes source consignés concordaient avec `provenance.json`; aucun secret lisible n'a été relevé. Aucun test ni service n'a été lancé par le réviseur.

**Durcissement recommandé puis réalisé en WEBHOOK-NATIVE-REPLAY-006 :** les hashes HTTP bruts du proxy et des `payload_content` Forgejo concordaient dans les artefacts 005, mais le runner n'assertait pas leur égalité. La fonction `validate_proxy_delivery_attempts` et un test négatif lient désormais chaque POST compté au task et à son hash brut; le parcours Forgejo v17 a été rejoué sous le numéro 006.

Réserves : le proxy implémente et compte le `POST` JSON configuré dans cet essai; les autres méthodes ne sont pas couvertes. La fenêtre sans replay manuel est d'une seconde. L'état privé reste sous `/tmp/forgejo-native-replay-005-state` avec permissions restreintes.
