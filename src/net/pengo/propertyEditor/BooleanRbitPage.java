/**
 * BooleanRbitPage.java
 *

 */

package net.pengo.propertyEditor;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;

import java.util.*;
import java.awt.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.text.*;
import javax.swing.event.*;
import java.io.IOException;
import net.pengo.pointer.JavaPointer;

/**
 *
 * @author  Peter Halasz
 */

public class BooleanRbitPage extends EditablePage {
    private BooleanAddressedResource res;
    private JTextField inputField;
    
    public BooleanRbitPage(BooleanAddressedResource res, AbstractResourcePropertiesForm form) {
        super(form);
        this.res = res;
        this.form = form;
	
        add(new JLabel( "Bit to use (0 for first on left): " ));
        inputField = new JTextField("0",12);
        inputField.addActionListener( getSaveActionListener() );
        inputField.getDocument().addDocumentListener( this );
	
	JButton pointerButton = new JButton("...");
	
        pointerButton.addActionListener(new ActionListener() {
		    
		    public void actionPerformed(ActionEvent e) {
			new ResourceSelectorForm(BooleanRbitPage.this.res.getRbitPointer()).show();
		    }
		});
	
        add(inputField);
	add(pointerButton);
        build();
    }
    
    
    public void saveOp() {
	if (isOwner()) {
	    try {
		res.getRbit().setValue(inputField.getText());
	    } catch (IOException e ) {
		//fixme
		e.printStackTrace();
	    }
	} else {
	    // it's already set then
	}
    }
    
    public void buildOp() {
	if (isOwner()) {
	    inputField.setText( res.getRbit().getValue() + "" );
	    inputField.setEnabled(true);
	} else {
	    //fixme: give more info on the pointer
	    inputField.setText( res.getRbit().getValue() + "" );
	    inputField.setEnabled(false);
	}
    }
    
    /** is the rbitpointer the owner of its pointed-to-thing? */
    private boolean isOwner() {
	JavaPointer rbp = res.getRbitPointer();
	boolean owner = rbp.getPrimarySource().isOwner(rbp);
	return owner;
    }
    
    public boolean isValid() {
        //fixme.. also fix IntValuePage
        return true; // success
    }
    
    public String toString() {
        return "Value";
    }
}

