import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;

class DefaultSelectionResource extends SelectionResource {
    public DefaultSelectionResource(OpenFile openFile, Data sel) {
	super(openFile, sel);
    }

    public JMenu getJMenu() {
        final SelectionResource This = this;
        
  	JMenu menu = new JMenu("Example");
        Action addToTemplate = new AbstractAction("Add to template") {
            public void actionPerformed(ActionEvent e) {
                DefaultDefinitionResource defRes = new DefaultDefinitionResource(openFile, sel);
                getOpenFile().addDefinition(this, defRes);
            }
        };
	menu.add(addToTemplate);
        
        final Data data = getOpenFile().getData();
        if (data instanceof DiffData) {
            Action delAction = new AbstractAction("Delete selected hex") {
                public void actionPerformed(ActionEvent e) {
                    getOpenFile().clearSelection(this);
                    ((DiffData)data).delete(sel.getStart(), sel.getLength());
                }
            };
            menu.add(delAction);

            Action insAction = new AbstractAction("Insert 00randomFF") {
                public void actionPerformed(ActionEvent e) {
                    getOpenFile().clearSelection(this);
                    ((DiffData)data).insert( sel.getStart(), new DemoData((int)sel.getLength())); //xxx: possible precision loss
                }
            };
            menu.add(insAction);

            Action replaceAction = new AbstractAction("replace with 00randomFF") {
                public void actionPerformed(ActionEvent e) {
                    getOpenFile().clearSelection(this);
                    ((DiffData)data).overwrite( sel.getStart(), new DemoData((int)sel.getLength())); //xxx: possible precision loss
                }
            };
            menu.add(replaceAction);
        }

        
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
