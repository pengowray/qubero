import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;

class DefinitionResource extends Resource {
    final protected RawDataSelection sel;
    public DefinitionResource(RawDataSelection sel) {
	super(sel.getOpenFile());
        this.sel = sel;
    }

    public JMenu getJMenu() {
  	JMenu menu = new JMenu("Example");
        Action addToTemplate = new AbstractAction("Delete") {
            public void actionPerformed(ActionEvent e) {
                getOpenFile().deleteDefinition(this, sel);
            }
        };
	menu.add(addToTemplate);
        
	menu.add(new JMenuItem("and stuff!"));
	//JPopupMenu popup = menu.getPopupMenu();
	return menu;
    }

    // how to respond to a rename event
    public void rename(String name) {
	//XXX
	return;
    }
    
    public String toString() {
        return sel.toString();
    }
    
    public void clickAction() {
        getOpenFile().setSelection(this, sel);
    }
    
}
