/*
 * BooleanResource.java
 *
 * Created on August 16, 2003, 10:50 PM
 */

package net.pengo.resource;

import java.io.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;
import java.util.*;
import java.math.*;
import java.io.IOException;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.propertyEditor.*;
import net.pengo.restree.ResourceList;

/**
 *
 * @author  Smiley
 */
abstract public class BooleanResource extends DefinitionResource {

    public BooleanResource(OpenFile of) {
	super(of);
    }

    abstract public boolean getValue();
    abstract public void setValue(boolean b);

    abstract public boolean isPrimative();
    
    public JMenu getJMenu() {
        //final OpenFile openFile = this.openFile;
        
  	JMenu menu = new JMenu("Menu");
        
        Action deleteAction = new AbstractAction("Delete") {
            public void actionPerformed(ActionEvent e) {
                //getOpenFile().deleteDefinition(e.getSource(), This);
		getOpenFile().getDefinitionList().remove(BooleanResource.this);
            }
        };
	menu.add(deleteAction);
        
	/*
        Action untypeAction = new AbstractAction("Convert to untyped definition") {
            public void actionPerformed(ActionEvent e) {
                //fixme: selRes may be null.
                DefaultDefinitionResource res = new DefaultDefinitionResource(openFile, selRes.getSelection());
                //openFile.definitionChange(e.getSource(), This, res);
		List l = getOpenFile().getDefinitionList();
		int index = l.indexOf(BooleanResource.this);
		l.remove(index);
		l.add(index, res);
		
            }
        };
	menu.add(untypeAction);
	 */
	
        Action propAction = new AbstractAction("Edit properties") {
            public void actionPerformed(ActionEvent e) {
                //fixme:
                new BooleanPrimativeResourcePropertiesForm(BooleanResource.this).show();
            }
        };
	menu.add(propAction);
	 
        //JPopupMenu popup = menu.getPopupMenu();
	return menu;
    }
    
    public ResourceList getSubResources() {
	// TODO
	return null;
    }
    
    abstract public void editProperties();

    
}
