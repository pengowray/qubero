/*
 * ValuePage.java
 *
 * Created on July 11, 2003, 11:51 PM
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
/**
 *
 * @author  Smiley
 */
public class ValuePage extends PropertyPage {
    private IntResource res;
    private IntResourcePropertiesForm form;
    private JTextField inputField;
    private boolean modded = false;
    private boolean updating = false; // if updating, ignore mods
    
    /** Creates a new instance of ValuePage */
    public ValuePage(IntResource res, IntResourcePropertiesForm form) {
        super();
        this.res = res;
        this.form = form;

        add(new JLabel( "Value: " ));
        inputField = new JTextField("0",12);
        inputField.addActionListener( new ActionListener() {
            public void actionPerformed(ActionEvent e) {
                ValuePage.this.form.save();
            }
        } );
        
        inputField.getDocument().addDocumentListener( new DocumentListener() {
            public void changedUpdate(DocumentEvent e) {
                mod();
            }
        
            public void insertUpdate(DocumentEvent e) {
                mod();
            }
            
            public void removeUpdate(DocumentEvent e) {
                mod();
            }
            
        });
        add(inputField);
        build();
    }
    
    private void mod() {
        if (!updating) {
            modded = true;
            form.mod();
        }
    }
    
    public void build() {
        updating = true;
        modded = false;
        inputField.setText(res.getValue().toString());
        updating = false;
    }
    
    public void save() {
        if (modded) {
            res.setValue(inputField.getText());
        }
    }
    
    public boolean isValid() {
        //fixme
        return true; // success
    }    
    
    public String toString() {
        return "Value";
    }    
}
