import javax.swing.AbstractAction;
import java.awt.event.ActionEvent;

class OpenAction extends AbstractAction {
    HexEditorGUI gui;
    public OpenAction(HexEditorGUI gui){
	super("Open");
	this.gui = gui;
    }
    public void actionPerformed(ActionEvent e) { 
	gui.open();
    }
}

