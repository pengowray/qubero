import javax.swing.AbstractAction;
import java.awt.event.ActionEvent;

class ExitAction extends AbstractAction {
    HexEditorGUI gui;
    public ExitAction(HexEditorGUI gui){
	super("Exit");
	this.gui = gui;
    }
    public void actionPerformed(ActionEvent e) { 
	gui.quit();
    }
}

