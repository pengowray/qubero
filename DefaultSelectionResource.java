import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;

//xxx: LiveSelectionResource?
class DefaultSelectionResource extends SelectionResource {
    public DefaultSelectionResource(OpenFile openFile) {
	super(openFile);
    }

    public JMenu getJMenu() {
        final SelectionResource This = this;
        
  	JMenu menu = new JMenu("Selection");
        Action addToTemplate = new AbstractAction("Add to template") {
            public void actionPerformed(ActionEvent e) {
                LongListSelectionModel selection = (LongListSelectionModel)(getOpenFile().getSelectionModel().clone());
                DefaultDefinitionResource def = new DefaultDefinitionResource(getOpenFile(), selection);
                getOpenFile().addDefinition(this, def);
            }
        };
	menu.add(addToTemplate);
        
        final Data data = getOpenFile().getData();
        if (data instanceof DiffData) {
            Action delAction = new AbstractAction("Delete selected hex") {
                public void actionPerformed(ActionEvent e) {
                    //getOpenFile().clearSelection(this);
                    //((DiffData)data).delete(sel.getStart(), sel.getLength());
                    LongListSelectionModel selection = getOpenFile().getSelectionModel();
                    getOpenFile().getEditableData().delete(selection);
                }
            };
            menu.add(delAction);

            //xxx: does not require a selection, just a lead selection index
            Action insAction = new AbstractAction("Insert 00randomFF (len:64)") {
                public void actionPerformed(ActionEvent e) {
                    //getOpenFile().clearSelection(this);
                    //((DiffData)data).insert( sel.getStart(), new DemoData((int)sel.getLength())); //xxx: possible precision loss
                    long pos = getOpenFile().getSelectionModel().getLeadSelectionIndex();
                    getOpenFile().getEditableData().insert(pos, new DemoData(64));
                }
            };
            menu.add(insAction);

            //xxx: put this back:
            /*
            Action replaceAction = new AbstractAction("replace with 00randomFF") {
                public void actionPerformed(ActionEvent e) {
                    getOpenFile().clearSelection(this);
                    ((DiffData)data).overwrite( sel.getStart(), new DemoData((int)sel.getLength())); //xxx: possible precision loss
                }
            };
            menu.add(replaceAction);
             */
            
            //xxx: paste
        }

        
        //JPopupMenu popup = menu.getPopupMenu();
	return menu;
    }
    
    /*
    //xxx: redundant for live selection. selection is already selected.
    public void doubleClickAction() {
        getOpenFile().setSelection(this, sel);
    }
     */

    // how to respond to a rename event
    public void rename(String name) {
	//XXX
	return;
    }
    
    //xxx:
    /*
    public String toString() {
        return sel.toString();
    }
     */
    
    
}
