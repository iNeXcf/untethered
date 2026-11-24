"""Two real Wayland windows on the fixture compositor. Change only the selected one."""
import sys
from PySide6 import QtCore, QtGui, QtWidgets

app=QtWidgets.QApplication([])
class Pattern(QtWidgets.QWidget):
    def __init__(self,title,colors):
        super().__init__();self.setWindowTitle(title);self.resize(480,270);self.colors=colors
    def paintEvent(self,_event):
        p=QtGui.QPainter(self)
        for i,c in enumerate(self.colors):p.fillRect(i*240,0,240,270,QtGui.QColor(c))
        p.end()
target=Pattern("Prism",["#eb2477","#0ecc99"])
other=Pattern("Decoy",["#ffff00","#ffff00"])
target.show();other.show()
def change():
    if sys.stdin.readline():
        target.colors=["#184fea","#f58421"];target.update()
notifier=QtCore.QSocketNotifier(0,QtCore.QSocketNotifier.Type.Read)
notifier.activated.connect(change)
sys.exit(app.exec())
