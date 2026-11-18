<div align="center">

<img src="assets/icon.svg" width="112" alt="ícone do Nino">

# Nino

**Grave a tela, tire prints e anote o que importa para a IA, tudo de uma vez.**<br>
Fork do [Ahri](https://github.com/juicerq/ahri) para KDE Plasma no Wayland, em Rust.

![Rust](https://img.shields.io/badge/Rust-edition_2024-1d1f25?style=flat-square&logo=rust&logoColor=white)
![KDE Plasma 6](https://img.shields.io/badge/KDE_Plasma-6-1d1f25?style=flat-square&logo=kde&logoColor=white)
![Wayland](https://img.shields.io/badge/Wayland-only-1d1f25?style=flat-square&logo=wayland&logoColor=white)

</div>

<br>

## Como funciona

1. <kbd>Win</kbd> <kbd>R</kbd> e arraste uma caixa. A gravação começa quando você solta o mouse.
2. Durante a gravação:
   - <kbd>Win</kbd> <kbd>N</kbd> abre uma anotação marcada com o tempo do vídeo.
   - <kbd>Win</kbd> <kbd>Shift</kbd> <kbd>S</kbd> tira um print com as ferramentas do Ahri e abre uma anotação para ele.
3. <kbd>Win</kbd> <kbd>R</kbd> de novo para parar.

Ao parar, o clipboard recebe o vídeo, os prints e o `anotacoes.md` como arquivos, e o texto organizado das anotações:

```text
Pasta: /home/voce/Vídeos/Nino/2026-11-18_14-03-22
Vídeo: gravacao.mp4

Anotação 1 - 02:32: "O **botão** salvar não responde"
Print 1 - 02:50 (print-1.png): "Erro aparece aqui"
Anotação 2 - 03:11: "Ver o `console`"
```

Colando num chat de IA que aceita arquivos, vão os arquivos. Colando num terminal ou editor, vai o texto. Tudo fica salvo em `~/Vídeos/Nino/`.

Fora de uma gravação, <kbd>Win</kbd> <kbd>Shift</kbd> <kbd>S</kbd> funciona como no Ahri: o print vai direto para o clipboard.

## Anotação

<p align="center"><img src="docs/anotacao.png" width="640" alt="Cartão de anotação com negrito, código e tarefas"></p>

O campo usa o editor do composer do [T3 Code](https://github.com/pingdotgg/t3code), só com a formatação: `**negrito**`, `*itálico*`, `~~riscado~~`, `` `código` `` e `- [ ] tarefa`. Os marcadores aparecem quando o cursor está no trecho, e o texto sai em Markdown.

| Tecla | Ação |
| --- | --- |
| <kbd>Enter</kbd> | Salvar |
| <kbd>Shift</kbd> <kbd>Enter</kbd> | Nova linha |
| <kbd>Ctrl</kbd> <kbd>Enter</kbd> | Salvar dentro de uma lista de tarefas |
| <kbd>Esc</kbd> | Cancelar a anotação ou pular o comentário do print |

## Print

<img src="docs/hero.png" alt="Overlay de print com traço, seta e barra de ferramentas">

| Tecla | Ação |
| --- | --- |
| <kbd>1</kbd> ou <kbd>A</kbd> | Desenhar |
| <kbd>2</kbd> ou <kbd>D</kbd> | Seta |
| <kbd>3</kbd> ou <kbd>Q</kbd> | Quadrado |
| <kbd>4</kbd> ou <kbd>E</kbd> | Círculo |
| <kbd>5</kbd> | Abrir as cores, depois <kbd>1</kbd>–<kbd>6</kbd> para escolher |
| <kbd>C</kbd> | Próxima cor |
| <kbd>Ctrl</kbd> <kbd>Z</kbd> | Desfazer |
| <kbd>Ctrl</kbd> <kbd>C</kbd> ou <kbd>Enter</kbd> | Copiar e fechar |
| <kbd>Esc</kbd> | Cancelar |

## Instalação

Precisa de KDE Plasma 6 no Wayland, Rust, [bun](https://bun.sh), [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/about/), gtk-layer-shell e WebKitGTK.

```sh
sudo pacman -S gpu-screen-recorder gtk-layer-shell webkit2gtk-4.1
git clone https://github.com/pedrinhopagan/nino
cd nino
./install.sh
```

O script instala o binário em `~/.local/bin`, registra o Nino no KWin e liga um serviço systemd de usuário. <kbd>Win</kbd> <kbd>Shift</kbd> <kbd>S</kbd> e <kbd>Win</kbd> <kbd>R</kbd> passam do Spectacle para o Nino; o Spectacle continua no <kbd>Print</kbd> e no <kbd>Win</kbd> <kbd>Shift</kbd> <kbd>R</kbd>.

<details>
<summary>Remover</summary>

```sh
systemctl --user disable --now nino
rm ~/.local/bin/nino ~/.local/share/applications/nino.desktop \
   ~/.local/share/icons/hicolor/scalable/apps/nino.svg ~/.config/systemd/user/nino.service
```

</details>

## Por baixo

| Parte | Como |
| --- | --- |
| Captura | `org.kde.KWin.ScreenShot2`, liberado pelo `.desktop` |
| Overlay | layer-shell com [smithay-client-toolkit](https://github.com/Smithay/client-toolkit) e [tiny-skia](https://github.com/linebender/tiny-skia) |
| Anotação | janela layer-shell com WebKitGTK e [Tiptap](https://tiptap.dev), aberta como subprocesso `nino editor` |
| Atalhos | kglobalaccel por D-Bus |
| Clipboard | `ext-data-control`, servido pelo próprio daemon |
| Gravação | `gpu-screen-recorder` em H.264, na GPU quando dá e na CPU quando não dá |

## Créditos

Base de captura, overlay e gravação: [Ahri](https://github.com/juicerq/ahri), do juicerq.<br>
Editor de anotação adaptado do composer do [T3 Code](https://github.com/pingdotgg/t3code), licença MIT em [`editor/LICENSE`](editor/LICENSE).
