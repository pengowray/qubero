/*
 * IntResource.java
 *
 * Created on 21 August 2002, 18:09
 */

import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;

/**
 *
 * @author  administrator
 */
public class IntResource extends DefinitionResource {
    // final protected RawDataSelection sel; // from Super
    /** Creates a new instance of IntResource */
    
    final static int UNSIGNED  = 0;
    final static int ONES_COMP = 1;
    final static int TWOS_COMP = 2;
    
    //XXX: little endian, big endian (2 byte only?)
    
    int length;
    int signed;
    RawDataSelection sel;
    
    public IntResource(RawDataSelection sel, int length, int signed) {
        super(sel.getOpenFile());
        if (sel.getLength() != length) {
            // wrong length. set length.
            // XXX: error checking!
            sel = new RawDataSelection(openFile, sel.getStart(), length);
        }
        this.sel = sel; // note: may be replaced as above
        this.length = length;
        this.signed = signed;
    }
    

    public JMenu getJMenu() {
        final IntResource This = this;
        final OpenFile openFile = this.openFile;
        
  	JMenu menu = new JMenu("Example");
        Action deleteAction = new AbstractAction("Delete") {
            public void actionPerformed(ActionEvent e) {
                getOpenFile().deleteDefinition(e.getSource(), This);
            }
        };
	menu.add(deleteAction);
        
        Action untypeAction = new AbstractAction("Convert to untyped definition") {
            public void actionPerformed(ActionEvent e) {
                DefaultDefinitionResource res = new DefaultDefinitionResource(sel);
                openFile.definitionChange(e.getSource(), This, res); // xxx
            }
        };
	menu.add(untypeAction);
        
	//JPopupMenu popup = menu.getPopupMenu();
	return menu;
    }
    
    public String toString() {
        return "int " + sel.toString();
    }
    
    public void clickAction() {
        getOpenFile().setSelection(this, sel);
    }    
}
