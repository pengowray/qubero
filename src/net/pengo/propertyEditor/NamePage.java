/*
 * ValuePage.java
 *
 * Created on July 11, 2003, 11:51 PM
 */

package net.pengo.propertyEditor;

import javax.swing.JLabel;
import javax.swing.JTextField;

import net.pengo.resource.Resource;
/**
 *
 * @author  Peter Halasz
 */
public class NamePage extends EditablePage {
    private Resource res;
    private JTextField inputField;
    
    /** Creates a new instance of ValuePage */
    public NamePage(Resource res, AbstractResourcePropertiesForm form) {
        super(form);
        this.res = res;
        this.form = form;

        add(new JLabel( "Name: " ));
        inputField = new JTextField("0",12);
        inputField.addActionListener( getSaveActionListener() );
        inputField.getDocument().addDocumentListener( this );
            
        add(inputField);
        build();
    }
    
    
    public void saveOp() {
        String name = inputField.getText();
        if (name==null || name.equals("")) {
            res.setName(null);
        } else {
            res.setName(name);
        }
    }
    
    public void buildOp() {
        String name = res.getName();
        if (name==null || name.equals("")) {
            inputField.setText("");
        } else {
            inputField.setText(name);
        }
    }
    
    public boolean isValid() {
        //fixme
        return true; // success
    }
    
    public String toString() {
        return "Name";
    }
}
