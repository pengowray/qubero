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

/**
 *
 * @author  Smiley
 */
public class BooleanResource extends DefinitionResource {
    private SelectionResource selRes;
    private IntResource rbit;
    private long bit; // use if IntResource is null.

    private boolean value; // use if selRes is null
    
    /** Creates a new instance of BooleanResource */
    public BooleanResource(OpenFile of, boolean value) {
        super(of);
        this.selRes = null;
        this.rbit = null;
        this.value = value;
    }
    public BooleanResource(OpenFile of, SelectionResource selRes, int bit) {
        super(of);
        this.selRes = selRes;
        this.rbit = null;
        this.bit = bit;
    }
    
    public BooleanResource(OpenFile of, SelectionResource selRes, IntResource rbit) {
        super(of);
        this.selRes = selRes;
        this.rbit = rbit;
    }
    
    public net.pengo.restree.ResourceList getSubResources() {
        return null;
    }
    
    public boolean getValue() throws IOException {
        if (selRes == null) 
            return value;
        
        
        long whichBit;
        
        if (rbit == null) {
            // use (int) bit 
            whichBit = bit;
        } else {
            whichBit = rbit.toLong();
        }
        
        long myByte = whichBit/8;
        int myBit = (int)(whichBit%8);

        SelectionData selData = selRes.getSelectionData();
        InputStream is = selData.dataStream();
        is.skip(myByte);
        byte[] valueByte = new byte[1];
        is.read(valueByte);

        boolean rValue = ((((int)valueByte[0]) & (1 << myBit)) >= 1);
        return rValue;
    }
    
    public void setValue(boolean b) {
        try {
            if (selRes == null) {
                value = b;
                return;
            }

            long whichBit;

            if (rbit == null) {
                // use (int) bit 
                whichBit = bit;
            } else {
                whichBit = rbit.toLong();
            }

            int myByte = (int)whichBit/8; //xxx: shuold be long :(
            int myBit = (int)(whichBit%8);

            SelectionData selData = selRes.getSelectionData();
            long start = selData.getStart();
            //byte[] valueBytes = selData.readByteArray(start, myByte+1); //xxx: fingers crossed
            byte[] valueBytes = selData.readByteArray(); //xxx: shouldn't take the whole thing you pig

            /*
            InputStream is = selData.dataStream();
            is.skip(myByte);
            byte[] valueByte = new byte[1];
            is.read(valueByte);        
            is.close();
            */

            if (b) {
                valueBytes[myByte] |= (1<<myBit);
            } else {
                valueBytes[myByte] &= ~(1<<myBit);
            }

            Data newdata = new ArrayData(valueBytes);
            //xxx: this is new.. must be changed in IntResource too
            openFile.getEditableData().insertReplace(selRes.getSelection(), newdata);
        } catch (java.io.IOException e) {
            //xxx
            e.printStackTrace();
            return;
        }
        
    }

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
        
        Action propAction = new AbstractAction("Edit properties") {
            public void actionPerformed(ActionEvent e) {
                //fixme:
                new BooleanResourcePropertiesForm(BooleanResource.this).show();
            }
        };
	menu.add(propAction);
	 
        //JPopupMenu popup = menu.getPopupMenu();
	return menu;
        
    }
}
