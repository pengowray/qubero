import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;

class SelectionResource extends Resource {
    final protected RawDataSelection sel; 
    public SelectionResource(RawDataSelection sel) {
	super(sel.getOpenFile());
        this.sel = sel;
    }

    public JMenu getJMenu() {
        final SelectionResource This = this;
        
  	JMenu menu = new JMenu("Example");
        Action addToTemplate = new AbstractAction("Add to template") {
            public void actionPerformed(ActionEvent e) {
                DefaultDefinitionResource defRes = new DefaultDefinitionResource(sel);
                getOpenFile().addDefinition(this, defRes);
            }
        };
	menu.add(addToTemplate);
        
	menu.add(new JMenuItem("and stuff!"));
	//JPopupMenu popup = menu.getPopupMenu();
	return menu;
    }
    
    public void clickAction() {
        getOpenFile().setSelection(this, sel);
    }

    // how to respond to a rename event
    public void rename(String name) {
	//XXX
	return;
    }
    
    public String toString() {
        return sel.toString();
    }
    
    
}
