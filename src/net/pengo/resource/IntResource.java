/*
 * IntResource.java
 *
 * Created on 21 August 2002, 18:09
 */

package net.pengo.resource;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.propertyEditor.*;
import net.pengo.restree.ResourceList;

import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;
import java.math.*;
import java.io.IOException;

/**
 *
 * @author  administrator
 */
abstract public class IntResource extends DefinitionResource {
    // final protected TransparentData sel; // from Super
    /** Creates a new instance of IntResource */
    
    public final static int UNSIGNED  = 0;
    public final static int ONES_COMP = 1;
    public final static int TWOS_COMP = 2;
    public final static int SIGN_MAG = 3;
    public final static int UNUSED_SIGN_BIT = 4; //fixme: NYI: Positive signed. e.g. 0-127 only

    //FIXME: little endian, big endian (2 byte only?), network byte order, local byte order
    
    //fixme: all these must be put in the subresource

    public IntResource(OpenFile of) {
	super(of);
    }
    
    abstract public boolean isPrimative();
    
    public JMenu getJMenu() {
        //final OpenFile openFile = this.openFile;
        
  	JMenu menu = new JMenu("Menu");
        
        Action deleteAction = new AbstractAction("Delete") {
            public void actionPerformed(ActionEvent e) {
                //getOpenFile().deleteDefinition(e.getSource(), This);
		getOpenFile().getDefinitionList().remove(IntResource.this);
            }
        };
	menu.add(deleteAction);
        
	//fixme: auto add for AddressedResources
        /*
	Action untypeAction = new AbstractAction("Convert to untyped definition") {
            public void actionPerformed(ActionEvent e) {
                //DefaultDefinitionResource res = new DefaultDefinitionResource(openFile, sel);
                DefaultDefinitionResource res = new DefaultDefinitionResource(openFile, selRes.getSelection()); // change to above when DefaultDefinitionResource is fixed
		List l = getOpenFile().getDefinitionList();
		int index = l.indexOf(IntResource.this);
		l.remove(index);
		l.add(index, res);
		
            }
        };
	menu.add(untypeAction);
	 */
        
        Action editAction = new AbstractAction("Edit value") {
            public void actionPerformed(ActionEvent e){
                BigInteger data = IntResource.this.getValue();
                new IntInputBox("Edit value","New value:",data.toString(),IntResource.this).show();
            }
        };
        menu.add(editAction);

        Action propAction = new AbstractAction("Edit properties") {
            public void actionPerformed(ActionEvent e) {
		editProperties();
            }
        };
	menu.add(propAction);
	 
        //JPopupMenu popup = menu.getPopupMenu();
	return menu;
        
    }
    
    /** bring up editor */
    abstract public void editProperties();

    public String toString() {
        return "int = " + getValue(); // sel.toString();
    }

    
    public long toLong() {
	return getValue().longValue();
    }

    abstract public BigInteger getValue();
    
    public void setValue(String value) throws NumberFormatException, IOException {
        setValue(new BigInteger(value));
    }
    
    abstract public void setValue(BigInteger value) throws IOException;
    
    
    /* no longer used */
    public static byte[] intToByte(int i) {
        //FIXME: this is retarded.
        //FIXME: use bigInteger ?
        
        byte[] b = new byte[4];
        b[0] = (byte) (i >> 24);
        b[1] = (byte) (i >> 16);
        b[2] = (byte) (i >> 8);
        b[3] = (byte) (i     );
        
        return b;
        //new ObjectInputStream(new DataOutputStream(
    }
    
    /* no longer used */
    public static int byteToInt(byte[] b) {
        //FIXME: this is also retarded.
        
        int i = ((0xff & b[0]) << 24) | ((0xff & b[1]) << 16) | ((0xff & b[2]) << 8) | (0xff & b[3]);
        
        return i;
    }
    
    public ResourceList getSubResources() {
        return null;
    }
    
}
