/*
 * DefaultDefinitionResource.java
 *
 * Created on 21 August 2002, 18:18
 */

import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;

/**
 *
 * @author  administrator
 */
public class DefaultDefinitionResource extends DefinitionResource {
    
    final protected LongListSelectionModel sel;

    public DefaultDefinitionResource(OpenFile openFile, LongListSelectionModel sel) {
	super(openFile);
        this.sel = sel;
    }

    public JMenu getJMenu() {
        final DefaultDefinitionResource This = this;
        final OpenFile openFile = this.openFile;
        
  	JMenu menu = new JMenu("Example");
        Action deleteAction = new AbstractAction("Delete") {
            public void actionPerformed(ActionEvent e) {
                getOpenFile().deleteDefinition(e.getSource(), This);
            }
        };
	menu.add(deleteAction);
        
        Action intAction = new AbstractAction("Convert to int") {
            public void actionPerformed(ActionEvent e) {
                //IntResource intRes = new IntResource(sel,4,IntResource.ONES_COMP);
                //IntResource intRes = new IntResource(openFile, sel,(int)sel.getLength(), IntResource.TWOS_COMP); //xxx possible precision loss
                IntResource intRes = new IntResource(openFile, sel, IntResource.TWOS_COMP);
                openFile.definitionChange(e.getSource(), This, intRes); // xxx
            }
        };
	menu.add(intAction);
        
        Action s2vAction = new AbstractAction("Set range to value...") {
            public void actionPerformed(ActionEvent e) {
                //new ByteEditor();
            }
        };
	menu.add(s2vAction);

        //JPopupMenu popup = menu.getPopupMenu();
	return menu;
    }
    
    public String toString() {
        return "untyped " + sel.toString();
    }
    
    public void doubleClickAction() {
        getOpenFile().setSelectionModel(sel);
    }
   
}
