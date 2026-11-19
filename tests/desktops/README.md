# Ensaios privados de desktop

Os runners usam diretórios HOME/XDG temporários e um D-Bus privado. Não recebem o display/bus do usuário. Não execute substituindo esses diretórios ou exportando sockets do desktop real. As capturas vêm somente do compositor de teste.

## Plasma no host

Dependências de teste: KWin, plasmashell, kscreen-doctor, Spectacle e FFmpeg. O binário já deve estar compilado.

```bash
scripts/run-limited.sh cargo build --locked -p waniengine-egui
scripts/run-limited.sh env WANIENGINE_TEST_SWAPS=25 WANIENGINE_TEST_HOTPLUG=1 scripts/test-egui-desktop.sh
scripts/run-limited.sh env WANIENGINE_TEST_CRASH=1 scripts/test-egui-desktop.sh
```

O runner imprime `/tmp/wani-backend-check.*` com relatório, logs, hashes e capturas. O caso de crash mata somente o PID que ele próprio criou, verifica seus descendentes e captura o fallback antes de reiniciar o manager privado. O caso de hotplug modifica somente os outputs virtuais do compositor privado.

Para loop prolongado de baixa carga, mantendo 100 trocas antes do loop:

```bash
scripts/run-limited.sh env WANIENGINE_TEST_SWAPS=100 WANIENGINE_SOAK_SECONDS=7200 scripts/test-egui-desktop.sh
```

`report.json` é escrito após a conclusão; a presença de `memory.jsonl` ou 100 linhas de Apply não significa que as duas horas terminaram. O script de coleta aceita `--desktop-pid` repetido para observar somente os PIDs conhecidos do desktop privado. Não use `--desktop` indiscriminadamente para um ensaio isolado, pois ele inclui os shells do mesmo usuário.

## GNOME/DING e Xfce em contêiner

A imagem contém desktops reais com renderização por software. Não há montagem de `/dev/dri`, display ou D-Bus do host. O DING oficial é fixado em 51.1.1 por hash e extraído somente em `target/desktop-matrix/vendor`; não entra no bundle do WaniEngine.

```bash
python3 tests/desktops/fetch-ding.py
docker build -t waniengine-desktop-tests:local -f tests/desktops/Dockerfile .
mkdir -p target/desktop-matrix
docker run --rm --init --ulimit core=0 --network none --memory 2g --memory-swap 2g --cpus 1 --pids-limit 256 --cap-drop ALL --security-opt no-new-privileges -e WANIENGINE_TEST_DING=1 -e WANIENGINE_TEST_SWAPS=25 --mount type=bind,source="$PWD",target=/workspace,readonly --mount type=bind,source="$PWD/target/desktop-matrix",target=/results waniengine-desktop-tests:local bash tests/desktops/run.sh gnome
docker run --rm --init --ulimit core=0 --network none --memory 1g --memory-swap 1g --cpus 0.5 --pids-limit 128 --cap-drop ALL --security-opt no-new-privileges --mount type=bind,source="$PWD",target=/workspace,readonly --mount type=bind,source="$PWD/target/desktop-matrix",target=/results waniengine-desktop-tests:local bash tests/desktops/run.sh xfce
```

A imagem executa como UID 1000. Em CI ou máquinas com UID diferente, o diretório descartável de resultados deve permitir escrita a esse usuário. Os resultados ficam em `target/desktop-matrix/{gnome,xfce}`. Xfce comprova recusa de operações ainda sem adaptador, não suporte a wallpaper.

`capture-extension` habilita a API de captura somente no Shell privado. Ela nunca é copiada pelo empacotador ou instalador. Não é uma extensão de produção do WaniEngine.

## Pacote candidato

```bash
python3 scripts/package-waniengine.py --egui --binary target/debug/waniengine-egui --output target/package-check
python3 tests/desktops/check-package.py target/package-check/WaniEngine-egui-0.2.0-linux-x86_64
python3 tests/test-installer.py
```

O ensaio do candidato usa caminhos temporários, verifica ausência de link Qt e licenças, instala/atualiza/desinstala e preserva a biblioteca. Não cria atalhos, autostart ou plugins na sessão real. Um binário debug empacotado serve para verificar payload/instalação; não substitui release otimizada, build na distro de base ou teste com Rust mínimo.
