/*
 * DefaultDefinitionResource.java
 *
 * Created on 21 August 2002, 18:18
 */

package net.pengo.resource;

import net.pengo.app.*;
import net.pengo.selection.*;

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
        final OpenFile openFile = this.openFile;
        
  	JMenu menu = new JMenu("Example");
        Action deleteAction = new AbstractAction("Delete") {
            public void actionPerformed(ActionEvent e) {
                //getOpenFile().deleteDefinition(e.getSource(), This);
		getOpenFile().getDefinitionList().remove(DefaultDefinitionResource.this);

            }
        };
	menu.add(deleteAction);
        
        Action intAction = new AbstractAction("Convert to int") {
            public void actionPerformed(ActionEvent e) {
                IntResource intRes = new IntResource(openFile, new DefaultSelectionResource(openFile, sel), IntResource.TWOS_COMP);
                //openFile.definitionChange(e.getSource(), This, intRes); // xxx
		List l = getOpenFile().getDefinitionList();
		int index = l.indexOf(DefaultDefinitionResource.this);
		l.remove(index);
		l.add(index, intRes);
            }
        };
	menu.add(intAction);

        Action boolAction = new AbstractAction("Convert to boolean") {
            public void actionPerformed(ActionEvent e) {
                BooleanResource boolRes = new BooleanResource(openFile, new DefaultSelectionResource(openFile, sel), 0);
                //openFile.definitionChange(e.getSource(), This, intRes); // xxx
		List l = getOpenFile().getDefinitionList();
		int index = l.indexOf(DefaultDefinitionResource.this);
		l.remove(index);
		l.add(index, boolRes);
            }
        };
	menu.add(boolAction);
        
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
        return "untyped: " + sel.toString();
    }
    
    public void doubleClickAction() {
        getOpenFile().setSelectionModel(sel);
    }
   
    public net.pengo.restree.ResourceList getSubResources() {
        return null;
    }
    
}
