from _window import window
from browser import jsstr

document = window.get_prop("document")
createElement = document.get_prop("createElement")
appendChild = document.get_prop("appendChild")

errorDiv = document.get_prop("getElementById").call(jsstr("error"), this=document)

for i in range(3):
    img = createElement.call(jsstr("img"), this=document)
    img.set_prop("src", jsstr("https://raw.githubusercontent.com/RustPython/RustPython/master/logo.png"))
    appendChild.call(img, this=errorDiv)
