/**
 * SimpleAddressPage.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;

import java.util.*;
import java.awt.*;
import javax.swing.*;
import javax.swing.text.*;
import javax.swing.event.*;
import java.awt.event.*;

public class SimpleAddressPage extends EditablePage {
    private AddressedResource res;
    private JTextField addressField;
    private JTextField lengthField;
    
    public SimpleAddressPage(AddressedResource res, AbstractResourcePropertiesForm form) {
	super(form);
	this.res = res;
	
	addressField = new JTextField("0",12);
	addressField.addActionListener( getSaveActionListener() );
	addressField.getDocument().addDocumentListener( this );
	
	lengthField = new JTextField("0",12);
	lengthField.addActionListener( getSaveActionListener() );
	lengthField.getDocument().addDocumentListener( this );
	
	add(addressField);
	add(lengthField);
	
	build();
    }
    
    public void buildOp() {
	SelectionResource sel = res.getSelectionResource();
	SelectionData selData = sel.getSelectionData();
	
	addressField.setText(selData.getStart()+"");
	lengthField.setText(selData.getLength()+"");
    }
    
    
    protected void saveOp() {
	long start = Long.parseLong( addressField.getText());
	long end = start + Long.parseLong( lengthField.getText()) - 1;
	OpenFile of = ((Resource)res).getOpenFile(); //fixme: bit ugly
	res.setSelectionResource(new DefaultSelectionResource(of, new SimpleLongListSelectionModel(start, end)));
    }
    
    
    public String toString() {
	return "Simple address";
    }
    
    
}

