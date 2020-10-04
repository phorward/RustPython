from browser import document
body = document.query("body")

for i in range(10):
    img = document.createElement("img")
    img.set_attr("src", "https://raw.githubusercontent.com/RustPython/RustPython/master/logo.png")
    body.appendChild(img)
